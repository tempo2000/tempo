//! Global keyboard hotkeys via direct `/dev/input/event*` capture.
//!
//! Wayland compositors don't let regular client apps grab arbitrary key
//! combos (only the compositor itself sees keystrokes destined for
//! other windows), so to support things like `Ctrl+Shift+Space` for
//! play/pause anywhere on the system we read the kernel's evdev
//! interface directly. This is the same approach that Solaar, OBS,
//! Discord, OpenRGB, etc. use.
//!
//! ## Permission model
//!
//! `/dev/input/event*` is `0660 root:input` on every modern distro,
//! so the user's shell account must be in the `input` group. Most
//! distros (Arch, Debian, Fedora, Omarchy) put interactive users in
//! `input` automatically; if not, the user runs
//! `sudo usermod -aG input $USER` once, logs out, logs back in. When
//! the open fails with `EACCES` we surface that in the Settings UI
//! with the exact command to fix it.
//!
//! ## Listen, don't grab
//!
//! We deliberately do NOT call `EVIOCGRAB` on the input devices. A
//! grabbed device delivers events ONLY to us, which would break every
//! other app on the system the moment Tempo starts. Instead we open
//! read-only and observe the same key stream the compositor sees.
//! The cost: when the user's bound combo (e.g. `Ctrl+Shift+Space`)
//! triggers, the focused app *also* sees the keystrokes (typically a
//! no-op there). Universally accepted tradeoff for global hotkeys on
//! Linux.
//!
//! ## Threading
//!
//! Each readable keyboard device gets its own dedicated OS thread
//! looping on `Device::fetch_events()`. Per-device modifier state is
//! tracked locally so chording across two keyboards (left-ctrl on
//! one, space on another) doesn't fire — that matches user
//! expectations and avoids pathological behavior on multi-keyboard
//! setups.
//!
//! Bindings are read through `Arc<RwLock<HotkeyConfig>>` and the
//! "currently recording" flag through `Arc<Mutex<RecordingSlot>>`.
//! Both are written from the GPUI thread (Settings → Hotkeys panel)
//! and read from every device thread on every keypress; reads
//! dominate, so `RwLock` is the right primitive.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::Result;
#[cfg(target_os = "linux")]
use anyhow::Context as _;
use crossbeam_channel::{Receiver, Sender};
#[cfg(target_os = "linux")]
use evdev::{Device, EventSummary, KeyCode};
use serde::{Deserialize, Serialize};

/// The set of player operations that can be bound to a global hotkey.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub enum HotkeyAction {
    PlayPause,
    NextTrack,
    PrevTrack,
    Stop,
    VolumeUp,
    VolumeDown,
    ToggleMute,
    PlayRandom,
    SeekForward,
    SeekBackward,
    CyclePlaybackMode,
    ShowWindow,
}

impl HotkeyAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::PlayPause => "Play / Pause",
            Self::NextTrack => "Next track",
            Self::PrevTrack => "Previous track",
            Self::Stop => "Stop",
            Self::VolumeUp => "Volume up",
            Self::VolumeDown => "Volume down",
            Self::ToggleMute => "Mute / Unmute",
            Self::PlayRandom => "Play random track",
            Self::SeekForward => "Seek forward 10s",
            Self::SeekBackward => "Seek backward 10s",
            Self::CyclePlaybackMode => "Cycle playback mode",
            Self::ShowWindow => "Show Tempo window",
        }
    }

    /// Stable id used as the key in `AppState::global_hotkeys`. Stable
    /// across builds so saved state survives version changes.
    pub fn storage_key(self) -> &'static str {
        match self {
            Self::PlayPause => "play_pause",
            Self::NextTrack => "next_track",
            Self::PrevTrack => "prev_track",
            Self::Stop => "stop",
            Self::VolumeUp => "volume_up",
            Self::VolumeDown => "volume_down",
            Self::ToggleMute => "toggle_mute",
            Self::PlayRandom => "play_random",
            Self::SeekForward => "seek_forward",
            Self::SeekBackward => "seek_backward",
            Self::CyclePlaybackMode => "cycle_playback_mode",
            Self::ShowWindow => "show_window",
        }
    }

    pub fn from_storage_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.storage_key() == key)
    }

    pub const ALL: [Self; 12] = [
        Self::PlayPause,
        Self::NextTrack,
        Self::PrevTrack,
        Self::Stop,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::ToggleMute,
        Self::PlayRandom,
        Self::SeekForward,
        Self::SeekBackward,
        Self::CyclePlaybackMode,
        Self::ShowWindow,
    ];
}

/// Modifier flags. Linux evdev exposes left/right variants for every
/// modifier; we collapse them into a single logical flag (the user
/// doesn't usually care whether they pressed left-ctrl or right-ctrl).
#[derive(Copy, Clone, Eq, PartialEq, Default, Debug, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn is_empty(self) -> bool {
        !(self.ctrl || self.shift || self.alt || self.meta)
    }

    fn render(self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("CTRL");
        }
        if self.alt {
            parts.push("ALT");
        }
        if self.shift {
            parts.push("SHIFT");
        }
        if self.meta {
            parts.push("SUPER");
        }
        parts.join("+")
    }
}

/// A bound key combo: zero-or-more modifiers plus exactly one main key.
/// The main key is stored as the kernel's evdev keycode (a `u16`) so
/// we don't have to round-trip through xkb keysyms — we're already
/// reading evdev codes anyway.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct KeyCombo {
    pub modifiers: Modifiers,
    /// evdev keycode of the main key (e.g. `KEY_SPACE` = 57).
    pub key: u16,
}

impl KeyCombo {
    /// Render combo as a human-readable string for the Settings UI and
    /// for `state.json` debug-readability. Format mirrors the typical
    /// "Ctrl+Shift+Space" convention but uses uppercase for the
    /// terminal key so it's visually distinct from the modifier list.
    pub fn display(&self) -> String {
        let mods = self.modifiers.render();
        let key = key_label(self.key);
        if mods.is_empty() {
            key.to_string()
        } else {
            format!("{mods}+{key}")
        }
    }
}

/// User's bound hotkeys. Wrapped in `RwLock` so the GPUI thread can
/// rebind a combo while the device threads are reading.
#[derive(Clone, Default, Debug)]
pub struct HotkeyConfig {
    bindings: HashMap<HotkeyAction, KeyCombo>,
}

impl HotkeyConfig {
    pub fn from_map(bindings: HashMap<HotkeyAction, KeyCombo>) -> Self {
        Self { bindings }
    }

    pub fn get(&self, action: HotkeyAction) -> Option<&KeyCombo> {
        self.bindings.get(&action)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&HotkeyAction, &KeyCombo)> {
        self.bindings.iter()
    }

    /// Find the action bound to the given combo, if any. Used by the
    /// device threads on every keypress; runs in O(N) which is fine
    /// (N <= 12).
    pub fn find_action(&self, combo: &KeyCombo) -> Option<HotkeyAction> {
        self.bindings.iter().find_map(
            |(action, bound)| {
                if bound == combo { Some(*action) } else { None }
            },
        )
    }
}

/// Recording slot: when `Some(action)`, the next non-modifier press
/// captures the current modifier state and is delivered back to the
/// UI via `recorded_tx` instead of dispatching as an action. The UI
/// flips this back to `None` either when it accepts the recording or
/// when the user cancels.
type RecordingSlot = Option<HotkeyAction>;

/// What the device threads emit. Either a normal hotkey activation
/// (the user pressed a bound combo) or a one-shot recording capture.
#[derive(Debug)]
pub enum HotkeyEvent {
    Activated(HotkeyAction),
    Recorded {
        action: HotkeyAction,
        combo: KeyCombo,
    },
}

/// Owns the device-watcher threads and the shared config. Drop the
/// service to signal all watcher threads to exit (they wake up at
/// most every `EVENT_BLOCK_TIMEOUT` apart).
pub struct HotkeyService {
    config: Arc<RwLock<HotkeyConfig>>,
    recording: Arc<RwLock<RecordingSlot>>,
    running: Arc<AtomicBool>,
    /// Initial scan result. `Ok(n)` means we successfully opened `n`
    /// keyboard devices; `Err(_)` means we couldn't read `/dev/input`
    /// at all (almost always EACCES → user not in `input` group).
    /// Surfaced unchanged in the Settings UI.
    pub init_status: InitStatus,
}

#[derive(Clone, Debug)]
pub enum InitStatus {
    /// At least one keyboard was opened; `count` is how many.
    /// `unreadable` is how many event devices we saw but couldn't
    /// open (typically because they're not keyboards or because of
    /// permission quirks on hot-plugged devices).
    Ok {
        keyboard_count: usize,
        unreadable: usize,
    },
    /// Couldn't open any device. The string is a user-facing
    /// explanation including the suggested fix command.
    PermissionDenied(String),
    /// The `/dev/input` directory itself couldn't be read.
    NoInputDir(String),
    /// We could read `/dev/input` and could enumerate event nodes,
    /// but found no keyboards.
    NoKeyboardsFound,
}

impl HotkeyService {
    /// Boot the watcher threads. Returns an event receiver that fires
    /// once per keypress that matches a bound combo (or, while
    /// recording, once with the captured combo).
    pub fn new(initial_config: HotkeyConfig) -> Result<(Self, Receiver<HotkeyEvent>)> {
        let config = Arc::new(RwLock::new(initial_config));
        let recording = Arc::new(RwLock::new(None));
        let running = Arc::new(AtomicBool::new(true));
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<HotkeyEvent>();

        let init_status = spawn_device_watchers(&config, &recording, &running, &event_tx);

        Ok((
            Self {
                config,
                recording,
                running,
                init_status,
            },
            event_rx,
        ))
    }

    /// Replace the bindings for one action. Pass `None` to clear.
    pub fn set_binding(&self, action: HotkeyAction, combo: Option<KeyCombo>) {
        let mut guard = self.config.write().expect("hotkeys config poisoned");
        match combo {
            Some(combo) => {
                guard.bindings.insert(action, combo);
            }
            None => {
                guard.bindings.remove(&action);
            }
        }
    }

    /// Snapshot the current bindings (for persistence).
    pub fn snapshot(&self) -> HashMap<HotkeyAction, KeyCombo> {
        self.config
            .read()
            .expect("hotkeys config poisoned")
            .bindings
            .clone()
    }

    /// Begin capturing the next physical key combo into `action`.
    /// While recording is active, **no** action dispatches happen
    /// (because the captured key is consumed for the recording),
    /// preventing weird UX where pressing the existing binding both
    /// fires the action and overwrites it.
    pub fn begin_recording(&self, action: HotkeyAction) {
        *self.recording.write().expect("recording lock poisoned") = Some(action);
    }

    /// Cancel an in-flight recording. Idempotent.
    pub fn cancel_recording(&self) {
        *self.recording.write().expect("recording lock poisoned") = None;
    }

    pub fn is_recording(&self) -> bool {
        self.recording
            .read()
            .expect("recording lock poisoned")
            .is_some()
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        // Threads exit on their own at next event-poll wakeup; we
        // intentionally don't `join` to avoid blocking a shutting-down
        // GPUI window for up to `EVENT_BLOCK_TIMEOUT`.
    }
}

/// How long a device read may block before yielding back to check the
/// `running` flag. Short enough that quitting feels instant.
const EVENT_BLOCK_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(not(target_os = "linux"))]
fn spawn_device_watchers(
    _config: &Arc<RwLock<HotkeyConfig>>,
    _recording: &Arc<RwLock<RecordingSlot>>,
    _running: &Arc<AtomicBool>,
    _event_tx: &Sender<HotkeyEvent>,
) -> InitStatus {
    // Global hotkeys on macOS/Windows would need entirely different
    // backends (CGEventTap / RegisterHotKey). Until those are wired
    // up, surface a clear "not available" status instead of failing.
    InitStatus::NoInputDir(
        "Global hotkeys are currently only supported on Linux (evdev). \
         macOS and Windows backends are not yet implemented."
            .to_string(),
    )
}

#[cfg(target_os = "linux")]
fn spawn_device_watchers(
    config: &Arc<RwLock<HotkeyConfig>>,
    recording: &Arc<RwLock<RecordingSlot>>,
    running: &Arc<AtomicBool>,
    event_tx: &Sender<HotkeyEvent>,
) -> InitStatus {
    let entries = match std::fs::read_dir("/dev/input") {
        Ok(entries) => entries,
        Err(error) => {
            return InitStatus::NoInputDir(format!(
                "Couldn't read /dev/input: {error}. Tempo's global \
                 hotkeys require read access to keyboard input devices."
            ));
        }
    };

    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("event") {
            paths.push(entry.path());
        }
    }
    paths.sort();

    let mut keyboard_count = 0usize;
    let mut unreadable = 0usize;
    let mut permission_denied = 0usize;

    for path in &paths {
        match Device::open(path) {
            Ok(device) => {
                if !is_keyboard(&device) {
                    continue;
                }
                let dev_path = path.clone();
                let config_clone = config.clone();
                let recording_clone = recording.clone();
                let running_clone = running.clone();
                let event_tx_clone = event_tx.clone();
                let thread_name = format!(
                    "tempo-hotkey-{}",
                    path.file_name().and_then(|s| s.to_str()).unwrap_or("evdev")
                );
                let spawn_result = thread::Builder::new().name(thread_name).spawn(move || {
                    run_device_watcher(
                        dev_path,
                        device,
                        config_clone,
                        recording_clone,
                        running_clone,
                        event_tx_clone,
                    );
                });
                if let Err(error) = spawn_result.context("spawning evdev watcher thread") {
                    crate::perf::event(
                        "hotkeys.device_thread_spawn_failed",
                        format!("path={} error={error:#}", path.display()),
                    );
                    continue;
                }
                keyboard_count += 1;
            }
            Err(error) => {
                // `PermissionDenied` is the dominant failure on a
                // misconfigured system; other I/O errors usually mean
                // the device disappeared mid-enumeration.
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    permission_denied += 1;
                } else {
                    unreadable += 1;
                }
            }
        }
    }

    if keyboard_count > 0 {
        return InitStatus::Ok {
            keyboard_count,
            unreadable: unreadable + permission_denied,
        };
    }

    if permission_denied > 0 {
        return InitStatus::PermissionDenied(
            "Tempo can't read /dev/input/event* (permission denied). \
             Add yourself to the `input` group with: \
             `sudo usermod -aG input $USER`, then log out and back in. \
             Alternatively grant the binary capability:  \
             `sudo setcap cap_dac_read_search+ep $(which tempo)`."
                .to_string(),
        );
    }

    InitStatus::NoKeyboardsFound
}

/// Heuristic: a "keyboard" is any evdev device that advertises support
/// for the standard letter keys. Some game-controllers and mice
/// expose `EV_KEY` for buttons but won't have e.g. `KEY_A`, so this
/// filter separates them from real keyboards. Matches the heuristic
/// used by `libinput`.
#[cfg(target_os = "linux")]
fn is_keyboard(device: &Device) -> bool {
    if let Some(supported) = device.supported_keys() {
        supported.contains(KeyCode::KEY_A) && supported.contains(KeyCode::KEY_Z)
    } else {
        false
    }
}

#[cfg(target_os = "linux")]
fn run_device_watcher(
    dev_path: PathBuf,
    mut device: Device,
    config: Arc<RwLock<HotkeyConfig>>,
    recording: Arc<RwLock<RecordingSlot>>,
    running: Arc<AtomicBool>,
    event_tx: Sender<HotkeyEvent>,
) {
    crate::perf::event(
        "hotkeys.device_watcher_start",
        format!("path={}", dev_path.display()),
    );
    // Per-device modifier state. We only OR-in mods coming from this
    // same device so cross-keyboard chords don't accidentally fire.
    let mut mods = Modifiers::default();

    // The evdev crate's `fetch_events` blocks indefinitely. To honor
    // our shutdown flag, set the underlying fd to non-blocking and
    // poll with a short sleep. evdev::Device exposes set_nonblocking
    // via `set_nonblocking(true)`.
    if let Err(error) = device.set_nonblocking(true) {
        crate::perf::event(
            "hotkeys.set_nonblocking_failed",
            format!("path={} error={error}", dev_path.display()),
        );
    }

    loop {
        if !running.load(Ordering::Acquire) {
            break;
        }

        match device.fetch_events() {
            Ok(events) => {
                for event in events {
                    if !running.load(Ordering::Acquire) {
                        return;
                    }
                    if let EventSummary::Key(_, code, value) = event.destructure() {
                        handle_key_event(code, value, &mut mods, &config, &recording, &event_tx);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // No events queued; sleep briefly and try again.
                thread::sleep(EVENT_BLOCK_TIMEOUT);
            }
            Err(error) => {
                // Most likely the device was unplugged. Log and exit
                // this watcher; the rest keep running.
                crate::perf::event(
                    "hotkeys.device_read_error",
                    format!("path={} error={error}", dev_path.display()),
                );
                break;
            }
        }
    }

    crate::perf::event(
        "hotkeys.device_watcher_exit",
        format!("path={}", dev_path.display()),
    );
}

#[cfg(target_os = "linux")]
fn handle_key_event(
    code: KeyCode,
    value: i32,
    mods: &mut Modifiers,
    config: &Arc<RwLock<HotkeyConfig>>,
    recording: &Arc<RwLock<RecordingSlot>>,
    event_tx: &Sender<HotkeyEvent>,
) {
    // value semantics from kernel evdev: 0 = release, 1 = press, 2 =
    // autorepeat. For modifiers we update on press AND release. For
    // main keys we only fire on initial press (no autorepeat), to
    // avoid spamming "Next Track" 30 times when the user holds the
    // bound key.
    let is_press = value == 1;
    let is_release = value == 0;

    // Update modifier state for both press and release. Holding both
    // L+R variants of the same modifier is normal; we treat the
    // modifier as "down" if either side is pressed. Tracking each
    // side separately would be overkill for a single-flag model.
    let modifier_change = match code {
        KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => {
            mods.ctrl = !is_release;
            true
        }
        KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => {
            mods.shift = !is_release;
            true
        }
        KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => {
            mods.alt = !is_release;
            true
        }
        KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => {
            mods.meta = !is_release;
            true
        }
        _ => false,
    };
    if modifier_change {
        return;
    }

    if !is_press {
        return; // ignore release & autorepeat for main keys
    }

    let combo = KeyCombo {
        modifiers: *mods,
        key: code.code(),
    };

    // Recording mode preempts dispatch. Reading inside its own scope
    // so we drop the read lock before sending on the channel.
    let recording_target = *recording.read().expect("recording lock poisoned");
    if let Some(action) = recording_target {
        // Refuse to record a bare modifier (we already filtered that
        // above) or a no-modifier single key like `K` -- those would
        // hijack typing globally. Require at least one modifier.
        if combo.modifiers.is_empty() {
            crate::perf::event(
                "hotkeys.recording_rejected_no_modifier",
                format!("key={}", combo.key),
            );
            return;
        }
        // Clear the slot first so a slow channel doesn't double-record.
        *recording.write().expect("recording lock poisoned") = None;
        let _ = event_tx.send(HotkeyEvent::Recorded { action, combo });
        return;
    }

    // Normal dispatch path: look up the binding and fire if matched.
    let cfg = config.read().expect("hotkeys config poisoned");
    if let Some(action) = cfg.find_action(&combo) {
        let _ = event_tx.send(HotkeyEvent::Activated(action));
    }
}

/// Map an evdev keycode to a short display label. We hand-roll this
/// rather than using xkbcommon because (a) we already have the
/// keycode, (b) we want stable, locale-independent labels in the
/// Settings UI ("SPACE" not " "), and (c) it's a closed set of keys
/// users actually bind. Falls back to `KEY_<n>` for unknown codes.
pub fn key_label(code: u16) -> String {
    // These are stable Linux kernel evdev keycodes (uapi/linux/input-event-codes.h).
    // Hard-coded so the function compiles on every platform; `evdev` is
    // a Linux-only dep (see Cargo.toml).
    let s = match code {
        // letters
        30 => "A", 48 => "B", 46 => "C", 32 => "D", 18 => "E", 33 => "F",
        34 => "G", 35 => "H", 23 => "I", 36 => "J", 37 => "K", 38 => "L",
        50 => "M", 49 => "N", 24 => "O", 25 => "P", 16 => "Q", 19 => "R",
        31 => "S", 20 => "T", 22 => "U", 47 => "V", 17 => "W", 45 => "X",
        21 => "Y", 44 => "Z",
        // digits (top row)
        11 => "0", 2 => "1", 3 => "2", 4 => "3", 5 => "4",
        6 => "5", 7 => "6", 8 => "7", 9 => "8", 10 => "9",
        // function keys
        59 => "F1", 60 => "F2", 61 => "F3", 62 => "F4", 63 => "F5",
        64 => "F6", 65 => "F7", 66 => "F8", 67 => "F9", 68 => "F10",
        87 => "F11", 88 => "F12",
        // navigation / editing
        57 => "SPACE", 28 => "ENTER", 15 => "TAB", 1 => "ESC",
        14 => "BACKSPACE", 110 => "INSERT", 111 => "DELETE",
        102 => "HOME", 107 => "END", 104 => "PAGEUP", 109 => "PAGEDOWN",
        103 => "UP", 108 => "DOWN", 105 => "LEFT", 106 => "RIGHT",
        // punctuation
        51 => ",", 52 => ".", 53 => "/", 39 => ";", 40 => "'",
        41 => "`", 12 => "-", 13 => "=", 26 => "[", 27 => "]", 43 => "\\",
        // media keys
        164 => "MEDIA_PLAYPAUSE", 163 => "MEDIA_NEXT", 165 => "MEDIA_PREV",
        166 => "MEDIA_STOP", 115 => "VOL_UP", 114 => "VOL_DOWN", 113 => "MUTE",
        _ => return format!("KEY_{code}"),
    };
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_storage_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for action in HotkeyAction::ALL {
            assert!(
                seen.insert(action.storage_key()),
                "duplicate for {action:?}"
            );
        }
    }

    #[test]
    fn from_storage_key_roundtrip() {
        for action in HotkeyAction::ALL {
            assert_eq!(
                HotkeyAction::from_storage_key(action.storage_key()),
                Some(action)
            );
        }
    }

    #[test]
    fn modifiers_render() {
        let m = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(m.render(), "CTRL+SHIFT");
        let m = Modifiers::default();
        assert_eq!(m.render(), "");
        let m = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            meta: true,
        };
        assert_eq!(m.render(), "CTRL+ALT+SHIFT+SUPER");
    }

    #[test]
    fn key_combo_display() {
        let combo = KeyCombo {
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
            key: 57, // KEY_SPACE
        };
        assert_eq!(combo.display(), "CTRL+SHIFT+SPACE");
    }

    #[test]
    fn key_combo_display_no_modifiers() {
        let combo = KeyCombo {
            modifiers: Modifiers::default(),
            key: 66, // KEY_F8
        };
        assert_eq!(combo.display(), "F8");
    }

    #[test]
    fn key_label_unknown_falls_back() {
        let label = key_label(9999);
        assert!(label.starts_with("KEY_"));
    }

    #[test]
    fn config_find_action() {
        let combo = KeyCombo {
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
            key: 57, // KEY_SPACE
        };
        let mut bindings = HashMap::new();
        bindings.insert(HotkeyAction::PlayPause, combo.clone());
        let cfg = HotkeyConfig::from_map(bindings);
        assert_eq!(cfg.find_action(&combo), Some(HotkeyAction::PlayPause));

        let other = KeyCombo {
            modifiers: Modifiers::default(),
            key: 45, // KEY_X
        };
        assert_eq!(cfg.find_action(&other), None);
    }
}
