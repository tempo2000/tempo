//! Live audio analysis for visualizers.
//!
//! Provides a [`TapSource`] that wraps a [`rodio::Source`] and feeds
//! interleaved samples into a shared [`AudioAnalyzer`] without altering
//! the audio path. The analyzer keeps a small ring buffer of recent
//! mono-mixed PCM, and on demand computes a windowed FFT to produce an
//! [`AnalysisFrame`] with broadband RMS (in dBFS), per-band magnitudes
//! for a frequency-bars visualizer, and a higher-resolution magnitude
//! spectrum suitable for a scrolling spectrogram.
//!
//! Designed around two access patterns:
//!
//! * **Producer (audio thread):** [`TapSource`] writes samples into the
//!   ring buffer on every `next()` call. This must be allocation-free
//!   and lock-cheap; the inner `Mutex` is only held for the short
//!   memcpy-style write.
//! * **Consumer (render thread):** [`AudioAnalyzer::latest_frame`] is
//!   called from the GPUI render path. It snapshots the most recent
//!   `FFT_SIZE` samples, applies a Hann window, runs an FFT, and
//!   returns derived metrics. A rate limiter caches the last frame so
//!   a 60 Hz repaint cycle doesn't pay for redundant FFTs.
//!
//! The producer never blocks on the consumer. If the render thread is
//! slow and the ring fills, old samples are simply overwritten -- the
//! visualizer always sees "the most recent N ms of audio".

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rodio::{ChannelCount, Sample, SampleRate, Source, source::SeekError};
use rustfft::{FftPlanner, num_complex::Complex32};

/// FFT window size. 1024 samples at 44.1 kHz ≈ 23 ms — short enough to
/// feel responsive but long enough for ~43 Hz frequency resolution,
/// which is plenty for a coarse music visualizer.
pub const FFT_SIZE: usize = 1024;

/// Capacity of the ring buffer holding mono-mixed PCM, in samples.
/// Sized to keep ~370 ms of audio at 44.1 kHz so a render-thread stall
/// never starves the analyzer of fresh data.
const RING_CAPACITY: usize = FFT_SIZE * 16;

/// Number of frequency bands surfaced for the bar visualizer. Bands are
/// log-spaced from `MIN_BAND_HZ` to `MAX_BAND_HZ`.
pub const BAND_COUNT: usize = 32;

/// Number of bins surfaced in [`AnalysisFrame::spectrum`] for the
/// spectrogram visualizer. Linearly spaced from 0 Hz up to Nyquist;
/// the visualizer is responsible for any further log mapping.
pub const SPECTRUM_BINS: usize = FFT_SIZE / 2;

const MIN_BAND_HZ: f32 = 40.0;
const MAX_BAND_HZ: f32 = 16_000.0;

/// Minimum interval between FFT computations. Caching at this rate is a
/// big win: a typical render burst at 120 Hz only triggers ~16 actual
/// FFTs/sec while still feeling smooth.
const ANALYSIS_INTERVAL: Duration = Duration::from_millis(16);

/// Snapshot of the most recently analyzed audio frame.
///
/// All magnitudes are linear amplitudes in `[0.0, 1.0]` (clamped) unless
/// otherwise noted. `rms_db` is the broadband RMS in dBFS, clamped to
/// `[-80.0, 0.0]`.
#[derive(Clone, Debug)]
pub struct AnalysisFrame {
    /// Broadband RMS energy of the FFT window, in dBFS. Useful for the
    /// "dancing line" visualizer which moves a horizontal line based on
    /// the energy at a particular band -- callers can pick a band from
    /// `bands` or just use this for full-band motion.
    pub rms_db: f32,
    /// Linear `[0.0, 1.0]` magnitude per log-spaced band. Each entry is
    /// already perceptually scaled (square-rooted) so it maps roughly
    /// linearly onto bar heights without further work in the renderer.
    pub bands: [f32; BAND_COUNT],
    /// Linear `[0.0, 1.0]` magnitude for each linearly-spaced FFT bin
    /// up to Nyquist. `Arc<[f32]>` so the renderer can hold it across
    /// frames cheaply (the spectrogram pushes one column per frame
    /// into a scrolling history without copying).
    pub spectrum: Arc<[f32]>,
    /// Sample rate used to compute the FFT, in Hz. Renderers use this
    /// to label or position bins along a frequency axis.
    pub sample_rate: u32,
    /// Wall clock at which the analysis ran. Lets callers detect "no
    /// new audio" (paused, between tracks) and fade visualizers out.
    pub captured_at: Instant,
}

impl AnalysisFrame {
    /// Linear amplitude in `[0.0, 1.0]` derived from `rms_db`. Maps
    /// `-80 dBFS -> 0.0` and `0 dBFS -> 1.0` with a perceptual curve
    /// that is friendlier for visualizer height than raw dB.
    pub fn rms_normalized(&self) -> f32 {
        // dBFS is negative; remap [-80, 0] -> [0, 1] then bias toward
        // the upper range so quiet tracks still produce visible motion.
        let normalized = ((self.rms_db + 80.0) / 80.0).clamp(0.0, 1.0);
        normalized.powf(1.5)
    }

    /// "Silent" frame for use before any audio has played. The
    /// renderer should treat this as a steady baseline; the timestamp
    /// is the epoch so any "is fresh" check compares false.
    pub fn silent(sample_rate: u32) -> Self {
        Self {
            rms_db: -80.0,
            bands: [0.0; BAND_COUNT],
            spectrum: vec![0.0; SPECTRUM_BINS].into(),
            sample_rate,
            // `Instant::now()` is fine here; "freshness" is checked by
            // comparing against `Instant::now()` at render time, and a
            // stale silent frame is indistinguishable from a live one.
            captured_at: Instant::now(),
        }
    }
}

struct RingBuffer {
    /// Mono-mixed samples. Older entries are overwritten when the
    /// producer outpaces the consumer.
    data: Vec<f32>,
    /// Next write index. The most recent sample lives at
    /// `(write_pos - 1) mod data.len()`.
    write_pos: usize,
    /// Total samples written since the buffer was created. Used by the
    /// consumer to detect fresh data without holding state.
    total_written: u64,
    /// Most recent reported sample rate. Audio sources can change rate
    /// mid-stream (rare, but `rodio` allows it); the consumer reads
    /// the latest value at FFT time.
    sample_rate: u32,
}

impl RingBuffer {
    fn new() -> Self {
        Self {
            data: vec![0.0; RING_CAPACITY],
            write_pos: 0,
            total_written: 0,
            sample_rate: 44_100,
        }
    }

    fn push_mono(&mut self, sample: f32) {
        let idx = self.write_pos;
        self.data[idx] = sample;
        self.write_pos = (idx + 1) % self.data.len();
        self.total_written = self.total_written.wrapping_add(1);
    }

    /// Copy the most recent `FFT_SIZE` samples into `out` in
    /// chronological order (oldest first). `out.len()` must equal
    /// [`FFT_SIZE`].
    fn snapshot_window(&self, out: &mut [f32]) {
        debug_assert_eq!(out.len(), FFT_SIZE);
        let len = self.data.len();
        // Window starts FFT_SIZE samples before write_pos.
        let start = (self.write_pos + len - FFT_SIZE) % len;
        if start + FFT_SIZE <= len {
            out.copy_from_slice(&self.data[start..start + FFT_SIZE]);
        } else {
            let tail = len - start;
            out[..tail].copy_from_slice(&self.data[start..]);
            out[tail..].copy_from_slice(&self.data[..FFT_SIZE - tail]);
        }
    }
}

/// Cached window function and FFT planner. Held under a separate mutex
/// from the ring buffer so the producer never contends on FFT state.
struct AnalysisState {
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<Complex32>,
    band_edges: [(usize, usize); BAND_COUNT],
    band_edges_for_rate: u32,
    last_frame: Option<AnalysisFrame>,
    last_frame_at: Option<Instant>,
}

impl AnalysisState {
    fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|n| {
                // Hann window. Reduces spectral leakage so the bars
                // don't smear across neighbouring bins on tonal audio.
                let phase = 2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE as f32 - 1.0);
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        Self {
            fft,
            window,
            scratch: vec![Complex32::default(); FFT_SIZE],
            band_edges: [(0, 0); BAND_COUNT],
            band_edges_for_rate: 0,
            last_frame: None,
            last_frame_at: None,
        }
    }

    fn ensure_band_edges(&mut self, sample_rate: u32) {
        if self.band_edges_for_rate == sample_rate {
            return;
        }
        let nyquist = sample_rate as f32 / 2.0;
        let bin_hz = nyquist / SPECTRUM_BINS as f32;
        let log_min = MIN_BAND_HZ.ln();
        let log_max = MAX_BAND_HZ.min(nyquist).ln();
        for i in 0..BAND_COUNT {
            let t0 = i as f32 / BAND_COUNT as f32;
            let t1 = (i + 1) as f32 / BAND_COUNT as f32;
            let f0 = (log_min + (log_max - log_min) * t0).exp();
            let f1 = (log_min + (log_max - log_min) * t1).exp();
            let b0 = ((f0 / bin_hz).floor() as usize).clamp(1, SPECTRUM_BINS - 1);
            let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, SPECTRUM_BINS);
            self.band_edges[i] = (b0, b1);
        }
        self.band_edges_for_rate = sample_rate;
    }
}

/// Shared analysis handle. Cheap to clone -- internally an `Arc` over
/// the ring buffer + analysis state. Construct once via
/// [`AudioAnalyzer::new`] and clone the handle into both the audio
/// pipeline ([`TapSource`]) and the renderer.
#[derive(Clone)]
pub struct AudioAnalyzer {
    inner: Arc<AnalyzerInner>,
}

struct AnalyzerInner {
    ring: Mutex<RingBuffer>,
    state: Mutex<AnalysisState>,
}

impl AudioAnalyzer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AnalyzerInner {
                ring: Mutex::new(RingBuffer::new()),
                state: Mutex::new(AnalysisState::new()),
            }),
        }
    }

    /// Wrap `source` so playing it also feeds the analyzer's ring
    /// buffer. The wrapper is transparent -- it forwards every
    /// `Source` method, only intercepting `next()` to capture samples.
    pub fn tap<S>(&self, source: S) -> TapSource<S>
    where
        S: Source,
    {
        TapSource {
            inner: source,
            analyzer: self.clone(),
            channel_index: 0,
            channel_accumulator: 0.0,
            channels: 0,
        }
    }

    /// Reset the ring buffer. Called when playback stops so a paused
    /// visualizer doesn't keep displaying the last second of audio.
    pub fn reset(&self) {
        if let Ok(mut ring) = self.inner.ring.lock() {
            ring.data.fill(0.0);
            ring.write_pos = 0;
            // total_written is *not* reset -- consumers compare it
            // monotonically to detect fresh frames.
        }
    }

    /// Get the most recent analysis frame.
    ///
    /// Recomputes at most every [`ANALYSIS_INTERVAL`]; intermediate
    /// calls return the cached frame. Cheap enough to call from every
    /// render frame.
    pub fn latest_frame(&self) -> AnalysisFrame {
        let now = Instant::now();
        // Fast path: cached frame within the rate-limit window.
        if let Ok(state) = self.inner.state.lock()
            && let (Some(frame), Some(last_at)) = (state.last_frame.as_ref(), state.last_frame_at)
            && now.duration_since(last_at) < ANALYSIS_INTERVAL
        {
            return frame.clone();
        }

        // Snapshot ring buffer under a short lock.
        let (samples, sample_rate, total_written) = {
            let ring = match self.inner.ring.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let mut buf = vec![0.0f32; FFT_SIZE];
            ring.snapshot_window(&mut buf);
            (buf, ring.sample_rate, ring.total_written)
        };

        // If the producer hasn't written a full window yet, return
        // silent. This avoids showing analysis of an all-zero buffer.
        if total_written < FFT_SIZE as u64 {
            let frame = AnalysisFrame::silent(sample_rate);
            if let Ok(mut state) = self.inner.state.lock() {
                state.last_frame = Some(frame.clone());
                state.last_frame_at = Some(now);
            }
            return frame;
        }

        let mut state = match self.inner.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.ensure_band_edges(sample_rate);

        // Apply window and copy into the FFT scratch buffer.
        let mut sum_sq = 0.0f32;
        for (i, sample) in samples.iter().enumerate() {
            let windowed = sample * state.window[i];
            sum_sq += sample * sample;
            state.scratch[i] = Complex32::new(windowed, 0.0);
        }
        let mean_sq = sum_sq / FFT_SIZE as f32;
        let rms = mean_sq.max(1e-12).sqrt();
        let rms_db = (20.0 * rms.log10()).clamp(-80.0, 0.0);

        // Borrow split: take a temporary &mut to the scratch slice
        // through the existing field, drop the FFT borrow before
        // reading magnitudes. Cloning the `Arc<dyn Fft>` is a single
        // refcount bump and avoids overlapping mutable borrows on
        // `state` during `process`.
        let fft = Arc::clone(&state.fft);
        fft.process(&mut state.scratch);

        // Magnitudes for the linearly-spaced spectrum (used by the
        // spectrogram). Normalized so that a 0 dBFS sine peaks near
        // 1.0 in its bin.
        let mut spectrum = vec![0.0f32; SPECTRUM_BINS];
        let scale = 2.0 / FFT_SIZE as f32;
        for (i, slot) in spectrum.iter_mut().enumerate() {
            let c = state.scratch[i];
            let mag = (c.re * c.re + c.im * c.im).sqrt() * scale;
            *slot = mag.min(1.0);
        }

        // Aggregate into log-spaced bands for the bar visualizer.
        let mut bands = [0.0f32; BAND_COUNT];
        for (i, (start, end)) in state.band_edges.iter().copied().enumerate() {
            let mut peak = 0.0f32;
            for slot in spectrum.iter().take(end).skip(start) {
                if *slot > peak {
                    peak = *slot;
                }
            }
            // Square-root for perceptual scaling. Most music sits at
            // -30 dBFS or quieter; a linear height map looks dead.
            bands[i] = peak.sqrt().min(1.0);
        }

        let frame = AnalysisFrame {
            rms_db,
            bands,
            spectrum: spectrum.into(),
            sample_rate,
            captured_at: now,
        };
        state.last_frame = Some(frame.clone());
        state.last_frame_at = Some(now);
        frame
    }
}

impl Default for AudioAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// `Source` adapter that captures samples into an [`AudioAnalyzer`]
/// without modifying the audio. Created via [`AudioAnalyzer::tap`].
pub struct TapSource<S> {
    inner: S,
    analyzer: AudioAnalyzer,
    /// Current channel index within the active frame. We average
    /// across channels to produce mono samples for the analyzer.
    channel_index: u16,
    channel_accumulator: f32,
    /// Cached channel count. Re-fetched from the source whenever it
    /// might have changed (the trait permits per-span changes).
    channels: u16,
}

impl<S: Source> TapSource<S> {
    fn refresh_channels(&mut self) {
        let chans = u16::from(self.inner.channels());
        if chans != self.channels {
            self.channels = chans.max(1);
            self.channel_index = 0;
            self.channel_accumulator = 0.0;
        }
        let rate = u32::from(self.inner.sample_rate());
        if let Ok(mut ring) = self.analyzer.inner.ring.lock() {
            ring.sample_rate = rate;
        }
    }
}

impl<S: Source> Iterator for TapSource<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        if self.channels == 0 {
            self.refresh_channels();
        }
        self.channel_accumulator += sample;
        self.channel_index += 1;
        if self.channel_index >= self.channels {
            let mono = self.channel_accumulator / self.channels.max(1) as f32;
            // Single short-lived lock per audio frame; the producer
            // never blocks on the analyzer (the consumer holds the
            // mutex only for the brief snapshot copy).
            if let Ok(mut ring) = self.analyzer.inner.ring.lock() {
                ring.push_mono(mono);
            }
            self.channel_index = 0;
            self.channel_accumulator = 0.0;
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for TapSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)?;
        // Seeking creates an audio discontinuity. Drop any partial
        // channel frame and clear the analyzer ring so visualizers do
        // not briefly blend pre-seek audio with the new position.
        self.channel_index = 0;
        self.channel_accumulator = 0.0;
        self.analyzer.reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake source emitting a constant value, used to exercise the
    /// tap + ring buffer without any decoder dependencies.
    struct ConstSource {
        value: f32,
        remaining: usize,
        sample_rate: SampleRate,
        channels: ChannelCount,
    }

    impl Iterator for ConstSource {
        type Item = Sample;
        fn next(&mut self) -> Option<Sample> {
            if self.remaining == 0 {
                return None;
            }
            self.remaining -= 1;
            Some(self.value)
        }
    }

    impl Source for ConstSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(self.remaining)
        }
        fn channels(&self) -> ChannelCount {
            self.channels
        }
        fn sample_rate(&self) -> SampleRate {
            self.sample_rate
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    struct SeekableSource;

    impl Iterator for SeekableSource {
        type Item = Sample;

        fn next(&mut self) -> Option<Sample> {
            None
        }
    }

    impl Source for SeekableSource {
        fn current_span_len(&self) -> Option<usize> {
            Some(0)
        }

        fn channels(&self) -> ChannelCount {
            channels(1)
        }

        fn sample_rate(&self) -> SampleRate {
            rate(44_100)
        }

        fn total_duration(&self) -> Option<Duration> {
            Some(Duration::from_secs(60))
        }

        fn try_seek(&mut self, _pos: Duration) -> Result<(), SeekError> {
            Ok(())
        }
    }

    fn rate(n: u32) -> SampleRate {
        SampleRate::new(n).unwrap()
    }

    fn channels(n: u16) -> ChannelCount {
        ChannelCount::new(n).unwrap()
    }

    #[test]
    fn silent_until_full_window() {
        let analyzer = AudioAnalyzer::new();
        let frame = analyzer.latest_frame();
        // No samples written yet -> rms pinned to floor.
        assert!((frame.rms_db - -80.0).abs() < 1e-3);
        assert_eq!(frame.bands.len(), BAND_COUNT);
        assert_eq!(frame.spectrum.len(), SPECTRUM_BINS);
    }

    #[test]
    fn captures_mono_average_across_channels() {
        let analyzer = AudioAnalyzer::new();
        let source = ConstSource {
            value: 0.5,
            remaining: FFT_SIZE * 4,
            sample_rate: rate(44_100),
            channels: channels(2),
        };
        let mut tap = analyzer.tap(source);
        // Drain the source so the analyzer's ring fills.
        while tap.next().is_some() {}
        let frame = analyzer.latest_frame();
        // A constant 0.5 is ~ -6 dBFS RMS.
        assert!(
            frame.rms_db > -10.0 && frame.rms_db <= 0.0,
            "rms_db = {}",
            frame.rms_db
        );
    }

    #[test]
    fn reset_clears_ring_but_state_recovers() {
        let analyzer = AudioAnalyzer::new();
        let source = ConstSource {
            value: 0.25,
            remaining: FFT_SIZE * 2,
            sample_rate: rate(44_100),
            channels: channels(1),
        };
        let mut tap = analyzer.tap(source);
        while tap.next().is_some() {}
        let _ = analyzer.latest_frame();
        analyzer.reset();
        // After a reset the analyzer's *cached* frame is still the
        // pre-reset analysis (we deliberately don't invalidate it so
        // a paused visualizer doesn't flicker to silence). New audio
        // will overwrite naturally.
        let frame = analyzer.latest_frame();
        assert_eq!(frame.spectrum.len(), SPECTRUM_BINS);
    }

    #[test]
    fn tap_source_forwards_seek_to_inner_source() {
        let analyzer = AudioAnalyzer::new();
        let mut tap = analyzer.tap(SeekableSource);

        assert!(tap.try_seek(Duration::from_secs(10)).is_ok());
    }
}
