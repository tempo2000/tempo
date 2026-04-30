use std::{fs::File, path::Path, path::PathBuf, sync::Arc, thread, time::Duration};

use anyhow::{Context, Result, anyhow};
use cpal::{Device, traits::DeviceTrait as _, traits::HostTrait as _};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};

use crate::audio_analyzer::AudioAnalyzer;
use crate::equalizer::{EqSource, EqState};
use crate::perf;

#[derive(Clone, Debug)]
pub struct PlaybackOutputDevice {
    pub name: String,
    pub is_default: bool,
}

/// Internal helper used by [`PlaybackController::output_devices`]
/// while filtering and deduping ALSA PCM aliases. Holds the raw
/// `pcm_id` (so we can prefer canonical entries like
/// `default:CARD=…`), the parsed card id, the human-readable
/// description name, and whether cpal flagged this entry as the
/// host's default output.
struct RawDevice {
    pcm_id: String,
    friendly: String,
    card: Option<String>,
    is_default: bool,
}

#[derive(PartialEq, Eq)]
enum GroupKey {
    /// ALSA PCM that bound to a specific card (e.g.
    /// `default:CARD=Generic`).
    Card(String),
    /// PCM with no `CARD=` clause (e.g. `pipewire`, `default`,
    /// `sysdefault`). Grouped by friendly name so two truly identical
    /// hostless entries collapse, while distinct ones (`pipewire`
    /// vs. `default`) stay separate when their description names
    /// differ.
    Friendly(String),
}

struct Group {
    key: GroupKey,
    is_default: bool,
    candidates: Vec<RawDevice>,
}

pub struct PlaybackController {
    _device: MixerDeviceSink,
    /// `Arc` so we can hand the player to background decode threads
    /// without blocking the UI on file open + `Decoder::try_from`.
    player: Arc<Player>,
    output_name: String,
    /// Live audio analyzer fed by a [`TapSource`] inserted between the
    /// decoder and the rodio sink. Cloned into the renderer for
    /// frequency-reactive visualizers (dancing line, bars, mini
    /// spectrogram). Surviving across `set_output` and `play_path` so
    /// the renderer's handle stays valid; only the *contents* of the
    /// ring buffer are reset on track change.
    analyzer: AudioAnalyzer,
    /// Shared 10-band EQ state. Cloned into the [`EqSource`] inserted
    /// in front of the rodio sink so the audio thread reads UI
    /// changes lock-free. Survives `set_output` and `play_path` so
    /// the equalizer settings persist across device switches and
    /// track changes.
    eq_state: EqState,
}

impl PlaybackController {
    pub fn new(preferred_output: Option<&str>, volume: f32, eq_state: EqState) -> Result<Self> {
        let _span = perf::span(
            "playback.new",
            format!("preferred_output={}", preferred_output.unwrap_or("default")),
        );
        let (device, output_name) = Self::open_output(preferred_output)?;
        let player = Arc::new(Player::connect_new(device.mixer()));
        player.set_volume(volume);

        Ok(Self {
            _device: device,
            player,
            output_name,
            analyzer: AudioAnalyzer::new(),
            eq_state,
        })
    }

    /// Handle to the live audio analyzer. Cheap to clone (`Arc`); the
    /// renderer keeps one and polls [`AudioAnalyzer::latest_frame`] on
    /// each repaint when a frequency-reactive visualizer is active.
    pub fn analyzer(&self) -> AudioAnalyzer {
        self.analyzer.clone()
    }

    /// Handle to the shared EQ state. Cheap to clone (`Arc`); the UI
    /// uses this to mutate band gains, preamp, and bypass without
    /// blocking the audio thread.
    pub fn eq_state(&self) -> EqState {
        self.eq_state.clone()
    }

    /// Build the user-facing list of audio output devices.
    ///
    /// On Linux/ALSA, `cpal::Host::output_devices()` returns one entry
    /// per ALSA PCM — including ALSA plugins (`samplerate`, `upmix`,
    /// `vdownmix`, `speexdsp`, …) and per-card channel-config aliases
    /// (`front:CARD=…`, `surround51:CARD=…`, `hw:…`, …). All of those
    /// share the same friendly `description.name()` per card, which
    /// is why the picker used to show "USB Audio Device" eight times
    /// in a row. They aren't distinct outputs — they're aliases for
    /// the same physical endpoint.
    ///
    /// Three-step cleanup:
    /// 1. Reject ALSA PCM plugins and channel-config aliases by
    ///    `pcm_id` prefix.
    /// 2. Dedupe surviving entries per ALSA card, preferring the
    ///    `default:CARD=…` PCM (PipeWire/Pulse default) over
    ///    `sysdefault:CARD=…` over the first remaining.
    /// 3. Disambiguate any cards that still produce identical
    ///    friendly labels by appending the card identifier.
    ///
    /// Finally sorted with the default device first, then alphabetic
    /// — matches the convention every desktop audio panel uses.
    pub fn output_devices() -> Vec<PlaybackOutputDevice> {
        let _span = perf::span("playback.output_devices", "");
        let host = cpal::default_host();
        let default_output_id = host
            .default_output_device()
            .and_then(|device| device.id().ok());

        let raw = match host.output_devices() {
            Ok(devices) => devices.collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        // Step 1: filter out ALSA PCM plugins & channel-config aliases
        // by `pcm_id` prefix. The list is intentionally conservative —
        // unknown prefixes (covering future / non-ALSA hosts) pass
        // through and are deduped in step 2.
        let mut surviving: Vec<RawDevice> = Vec::new();
        for device in raw {
            let device_id = device.id().ok();
            // `DeviceId` is `(HostId, pcm_id)`; `.1` is the raw ALSA
            // PCM string we need for prefix filtering and `CARD=…`
            // parsing. Used in place of the deprecated `Device::name()`.
            let pcm_id = device_id
                .as_ref()
                .map(|id| id.1.clone())
                .unwrap_or_default();
            if Self::is_pcm_plugin(&pcm_id) {
                perf::event(
                    "playback.output_devices.filtered",
                    format!("pcm_id={pcm_id} reason=plugin_or_alias"),
                );
                continue;
            }
            let friendly = Self::device_name(&device);
            let card = Self::extract_card_id(&pcm_id);
            let is_default = device_id == default_output_id;
            surviving.push(RawDevice {
                pcm_id,
                friendly,
                card,
                is_default,
            });
        }

        // Step 2: group by card (or by friendly name when no
        // `CARD=…` is present, e.g. the bare `pipewire` /
        // `default` / `sysdefault` PCMs that PipeWire/Pulse expose).
        // Within a group, prefer `default:CARD=…`, then
        // `sysdefault:CARD=…`, then first remaining. The `is_default`
        // flag is OR'd across the group so the chosen survivor
        // inherits it from whichever PCM cpal flagged.
        let mut groups: Vec<Group> = Vec::new();
        for device in surviving {
            let group_key = device
                .card
                .clone()
                .map(GroupKey::Card)
                .unwrap_or_else(|| GroupKey::Friendly(device.friendly.clone()));
            if let Some(group) = groups.iter_mut().find(|g| g.key == group_key) {
                group.is_default |= device.is_default;
                group.candidates.push(device);
            } else {
                groups.push(Group {
                    key: group_key,
                    is_default: device.is_default,
                    candidates: vec![device],
                });
            }
        }

        let mut chosen: Vec<PlaybackOutputDevice> = groups
            .into_iter()
            .map(|group| {
                let candidates = &group.candidates;
                let pick = candidates
                    .iter()
                    .find(|d| d.pcm_id.starts_with("default:"))
                    .or_else(|| {
                        candidates
                            .iter()
                            .find(|d| d.pcm_id.starts_with("sysdefault:"))
                    })
                    .unwrap_or(&candidates[0]);
                PlaybackOutputDevice {
                    name: Self::display_label(pick),
                    is_default: group.is_default,
                }
            })
            .collect();

        // Step 3: disambiguate any remaining duplicate friendly labels.
        // Two physically distinct cards with identical model strings
        // (e.g. two of the same USB DAC) would otherwise render the
        // same name. Append a numeric suffix to all but the first
        // occurrence so each entry is uniquely selectable.
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for device in chosen.iter_mut() {
            let count = seen.entry(device.name.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                device.name = format!("{} ({})", device.name, *count);
            }
        }

        // Step 4: sort. Default first, then alphabetical.
        chosen.sort_by(|a, b| match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        chosen
    }

    /// Returns true for ALSA `pcm_id`s that represent plugins or
    /// channel-config aliases rather than user-facing output endpoints.
    /// Conservative — unknown prefixes pass through.
    fn is_pcm_plugin(pcm_id: &str) -> bool {
        // Normalize: ALSA pcm ids look like `front:CARD=Generic,DEV=0`
        // or just `pipewire` / `pulse`. We compare by the prefix
        // before the first `:`.
        let prefix = pcm_id.split(':').next().unwrap_or(pcm_id);
        const PLUGIN_PREFIXES: &[&str] = &[
            // Pure-software ALSA plugins (sample-rate converters, EQ,
            // upmix/downmix, denoisers, null sinks).
            "null",
            "samplerate",
            "speexrate",
            "lavrate",
            "speexdsp",
            "speex",
            "upmix",
            "vdownmix",
            // IPC plugins. On a PipeWire system the real default
            // endpoint is exposed as `pipewire` (and as the bare
            // `default` PCM), so these aliases are redundant.
            "oss",
            "jack",
            "pulse",
            // Per-card channel-config aliases. All map to the same
            // physical card; the canonical `default:CARD=…` /
            // `sysdefault:CARD=…` covers them.
            "hdmi",
            "iec958",
            "spdif",
            "front",
            "rear",
            "center_lfe",
            "side",
            "surround21",
            "surround40",
            "surround41",
            "surround50",
            "surround51",
            "surround71",
            // Low-level direct-hardware aliases.
            "dmix",
            "dsnoop",
            "hw",
            "plughw",
        ];
        PLUGIN_PREFIXES.contains(&prefix)
    }

    /// Extract the ALSA card identifier from a `pcm_id` like
    /// `default:CARD=Generic,DEV=0`. Returns `None` for hostless
    /// PCMs (`pipewire`, `default`, `sysdefault`) that don't bind to
    /// a specific card.
    fn extract_card_id(pcm_id: &str) -> Option<String> {
        let after_colon = pcm_id.split(':').nth(1)?;
        let card_eq = after_colon
            .split(',')
            .find(|frag| frag.starts_with("CARD="))?;
        let value = card_eq.trim_start_matches("CARD=");
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }

    fn display_label(device: &RawDevice) -> String {
        device.friendly.clone()
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn set_volume(&self, volume: f32) {
        self.player.set_volume(volume);
    }

    pub fn set_output(&mut self, output_name: &str, volume: f32) -> Result<()> {
        let _span = perf::span("playback.set_output", format!("output={output_name}"));
        let (device, output_name) = Self::open_output(Some(output_name))?;
        let player = Arc::new(Player::connect_new(device.mixer()));
        player.set_volume(volume);

        self.player.stop();
        // Clear visualizer history; the new device will start writing
        // fresh samples once playback resumes.
        self.analyzer.reset();
        self._device = device;
        self.player = player;
        self.output_name = output_name;
        Ok(())
    }

    fn open_output(preferred_output: Option<&str>) -> Result<(MixerDeviceSink, String)> {
        let _span = perf::span(
            "playback.open_output",
            format!("preferred_output={}", preferred_output.unwrap_or("default")),
        );
        let host = cpal::default_host();
        let outputs = host
            .output_devices()
            .context("failed to list audio outputs")?
            .collect::<Vec<_>>();
        let output = preferred_output
            .and_then(|preferred| {
                outputs
                    .iter()
                    .find(|output| Self::device_name(output) == preferred)
                    .cloned()
            })
            .or_else(|| host.default_output_device())
            .or_else(|| outputs.first().cloned())
            .context("no audio output device available")?;
        let output_name = Self::device_name(&output);
        let device = DeviceSinkBuilder::from_device(output)
            .context("failed to prepare audio output device")?
            .open_stream()
            .context("failed to open audio output device")?;

        Ok((device, output_name))
    }

    fn device_name(device: &Device) -> String {
        device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_else(|_| "Unknown output".to_string())
    }

    /// Begin playback of `path` without blocking the calling thread on
    /// file open + decoder construction. Stops the current source
    /// synchronously (so the previous track is silenced immediately) and
    /// hands off the slow `Decoder::try_from` work to a short-lived
    /// worker thread that calls `player.append` once the decoder is ready.
    ///
    /// The previous synchronous implementation could spend tens to
    /// hundreds of milliseconds parsing FLAC/MP3 headers on the UI
    /// thread; this version returns near-instantly so click-to-audio
    /// latency no longer hitches the rendering loop.
    pub fn play_path(&self, path: &Path) -> Result<()> {
        let _span = perf::span("playback.play_path", format!("path={}", path.display()));
        // Stop synchronously so the *previous* source is silenced before
        // we return to the caller; the *next* source's decode work
        // happens off-thread.
        self.player.stop();
        // Clear the analyzer's ring buffer so visualizers don't briefly
        // display the *previous* track's tail while the new decoder
        // spins up.
        self.analyzer.reset();

        let player = Arc::clone(&self.player);
        let analyzer = self.analyzer.clone();
        let eq_state = self.eq_state.clone();
        let path: PathBuf = path.to_path_buf();
        thread::Builder::new()
            .name("tempo-decode".into())
            .spawn(move || {
                let _span = perf::span("playback.decode_async", format!("path={}", path.display()));
                let file = match File::open(&path) {
                    Ok(file) => file,
                    Err(error) => {
                        perf::event(
                            "playback.decode_async.open_error",
                            format!("path={} error={error}", path.display()),
                        );
                        return;
                    }
                };
                let source = match Decoder::try_from(file) {
                    Ok(source) => source,
                    Err(error) => {
                        perf::event(
                            "playback.decode_async.decode_error",
                            format!("path={} error={error}", path.display()),
                        );
                        return;
                    }
                };
                // Source pipeline:
                //
                //   Decoder -> TapSource (analyzer) -> EqSource -> Player
                //
                // The analyzer taps *before* EQ so visualizers reflect
                // the original signal (visualizers "watch the song",
                // not the user's EQ adjustments). EQ is the last
                // step before the mixer so adjustments take effect
                // immediately and uniformly.
                let tapped = analyzer.tap(source);
                let eq = EqSource::new(tapped, eq_state);
                player.append(eq);
                player.play();
            })
            .context("failed to spawn audio decode thread")?;

        Ok(())
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn resume(&self) {
        self.player.play();
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    pub fn seek(&self, position: Duration) -> Result<()> {
        self.player
            .try_seek(position)
            .map_err(|error| anyhow!("failed to seek playback: {error}"))
    }

    pub fn is_empty(&self) -> bool {
        self.player.empty()
    }
}
