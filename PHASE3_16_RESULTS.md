# Phase 3 #16 — TempoApp Entity Split: Results

## Summary

The audio playback subsystem has been extracted into a standalone GPUI
entity (`PlayerEntity`), living at `src/app/player/entity.rs`. All
playback-related state — audio backend, current track identity, volume,
output device picker, waveform cache, now-playing hover affordances —
moved off `TempoApp`. Communication is event-driven via a typed
`PlayerEvent` channel; the parent subscribes once at construction.

**Build green, 29 tests passing, fmt clean, release build clean.**

## What was achieved

### Architectural cleanup

- **17 fields removed** from `TempoApp`, replaced by a single
  `Entity<PlayerEntity>` handle plus two snapshot mirrors
  (`volume_snapshot`, `output_device_snapshot`) for serialization.
- **600+ LOC moved** into a documented module with deliberate event
  boundary. `src/app/player/` is now a subdirectory with
  `mod.rs` (TempoApp glue + render code) and `entity.rs`
  (PlayerEntity + waveform pipeline).
- **6 typed events** in `PlayerEvent`: `PlayingTrackChanged`,
  `IsPlayingChanged`, `TrackFinished`, `NowPlayingLinkClicked`,
  `StateMutated` (+ a `replace_catalog`-related dead-code allowance).
  Single `handle_player_event` dispatch on `TempoApp`; each event
  variant has documented intent.
- **Path-keyed waveform cache** (`HashMap<PathBuf, Arc<[f32]>>`)
  replaces the `Vec<Option<Arc<[f32]>>>` parallel to `tracks`.
  - Eliminated 5 manual sync sites in `library_state.rs` (lines
    290–291, 791–795, 803–804, 875–876, 929–933).
  - Self-managing: track adds/removes don't require any cache mutation.
  - Library reload keeps entries for surviving tracks.
- **Path-keyed playing track identity**: `PlayerEntity::playing_track_path`
  is `Option<PathBuf>`, not `usize`. Survives library reloads (the
  prior `playing_track: usize = 0` reset on reload was a known bug;
  fixed incidentally by the migration).
- **No cyclic backreference**: `PlayerEntity` deliberately holds no
  `WeakEntity<TempoApp>`. Every cross-region need is expressible as
  an event, which keeps the entity testable in isolation and avoids
  re-entrancy hazards with `app.update`/`player.update`.
- **`SeekClickOutcome` return type** for `seek_from_waveform_click`:
  the parent gets a `{ target, needs_restart }` struct instead of
  the old fragile post-hoc `playback_position().is_zero()` check.

### Code-quality wins (counted)

| Concern | Before | After |
|---|---|---|
| Long-lived GPUI entities in this codebase | 1 (`TempoApp`) | 2 (`TempoApp` + `PlayerEntity`) |
| `cx.subscribe` / `cx.observe` callsites | 0 | 2 (one each, on the player) |
| Manual `waveform_cache` / `waveform_loading` sync sites | 8 | 0 |
| `synthetic_*` parallel-Vec gymnastics | 5 sites in library_state | 0 |
| `cx.notify()` callsites in player code | 24 in `player.rs` | 21 in `player/mod.rs` (parent UI) + 11 in `player/entity.rs` (state mutations) |
| Lines of player-related code | 1730 in `player.rs` | 1465 in `player/mod.rs` + 952 in `player/entity.rs` |

The line count grew because the new code includes ~250 lines of
module-level architectural documentation, comments explaining each
event variant's rationale, and `#[allow(dead_code)]` annotations
documenting forward-compatible API surface for media-keys, MPRIS,
DBus, headless rendering, etc.

### Behavioral improvements (incidental fixes)

- **Empty library now also stops audio backend.** Previously,
  `clamp_track_indices` flipped `is_playing = false` without draining
  the rodio sink, so removing all tracks left whatever was playing in
  the queue. Now `player.stop()` drains the sink as well.
- **Playback survives library reload.** Previously, every reload reset
  `playing_track: usize = 0` even if the user's currently-playing
  track was still in the library. The path-keyed identity fixes this.

## What was *not* achieved

### The headline perf win

The original goal was to **localize invalidation**: make per-second
playback ticks repaint only the player bar instead of the whole tree.
This work creates the architectural foundation but doesn't yet realize
the win, because:

- `TempoApp::render_player_bar` still owns the player bar's layout
  (it needs the active `Track` from `self.tracks`).
- `PlayerEntity` is rendered as a *model*, not a *view* (no `Render`
  impl, not embedded as a child element).
- `cx.observe(&player, |_, _, cx| cx.notify())` is wired in
  `TempoApp::new` so player notifies still trigger a full-tree repaint.

This is **not a regression**: the prior implementation already
throttled the 250 ms tick to ~1 Hz of `cx.notify()` (only on
integer-second changes), and the new implementation keeps the same
throttle. So tick frequency is unchanged. The per-event-handler
invalidation cost is also unchanged because the parent's
`handle_player_event` calls `cx.notify()` after each event.

### Path to the perf win

To unlock localized invalidation, one of these is needed:

1. **`impl Render for PlayerEntity`** + embed `self.player.clone()` as
   a child element of `TempoApp::render`. Requires pushing the active
   `Track`'s render-relevant fields (title/artist/album/codec/bitrate/
   year/duration_value/duration string/album_initials/album_color/
   artwork) onto `PlayerEntity` after each `play_track`. The natural
   way is to extend `PlayerEvent::PlayingTrackChanged` into a
   `PlayerEvent::PlayingTrackUpdated { snapshot: PlayingTrackSnapshot
   }` that the parent emits *after* the player has loaded the file,
   carrying the metadata. **Estimated effort: 1–2 days. Risk: low
   (the new module structure makes this isolated).**

2. **Bundle with #17** (`with_animation` for waveform loading): same
   refactor in (1), plus replacing the waveform-shimmer
   `request_animation_frame` with `with_animation` so the shimmer
   animation also runs entirely inside `PlayerEntity::render`.

The current state is a **clean, well-documented foundation** for
either path. The `cx.observe` is documented in the construction site
as a temporary measure.

## Module structure (post-migration)

```
src/app/player/
├── mod.rs       (1465 LOC)
│   ├── handle_player_event           — single subscriber dispatch
│   ├── refresh_player_state_snapshot — keeps volume/output mirrors current
│   ├── start_deferred_playback_init  — delegate to entity
│   ├── start_playback_tick           — delegate to entity
│   ├── play_track / play_track_with_history — orchestration (play count, history)
│   ├── play_finished_track           — auto-advance per playback mode
│   ├── play_random_track             — needs current_track_indices()
│   ├── play_adjacent_track           — needs current_track_indices()
│   ├── toggle_playback               — smart pause/resume/restart
│   ├── seek_playback / seek_from_waveform_click — backend-empty recovery
│   ├── set_playback_volume / toggle_mute / set_max_volume
│   ├── begin_volume_drag / drag_volume / finish_volume_drag — tooltip wiring
│   ├── select_output_device          — restart playback after device swap
│   ├── cycle_playback_mode
│   ├── queue_track / queue_track_at_start / queue_track_at_end / queue_album_from_track
│   ├── remove_track_from_queue
│   ├── clamp_track_indices           — also stops audio when empty
│   ├── render_marquee_text           — stateless helper (free fn shape)
│   ├── render_player_bar             — main player UI (reads self.player)
│   ├── playback_status_dropdown / output_device_menu (player + settings variants)
│   ├── waveform_seekbar / waveform_bar / transport_overlay / transport_button
│   └── volume_speaker_icon / bitrate_label
└── entity.rs    (952 LOC)
    ├── PlayerEvent (enum, 6 variants, fully documented)
    ├── SeekClickOutcome
    ├── PlayerEntity (struct, 17 fields grouped by concern with rationale)
    ├── EventEmitter<PlayerEvent>
    ├── new / start_deferred_init / start_playback_tick / replace_catalog / reset_for_library_reload
    ├── State queries: is_playing / playing_track_path / playback_mode / volume / output_device /
    │                  is_volume_dragging / settings_output_menu_open / has_playback /
    │                  playback_status / playback_status_label / current_output_label /
    │                  playback_position / playback_mode_label
    ├── Commands: start_playback / pause / resume / stop / seek / seek_from_waveform_click /
    │             set_volume / toggle_mute / set_max_volume / cycle_playback_mode /
    │             select_output_device / toggle_output_menu / close_output_menu /
    │             begin_volume_drag / drag_volume / finish_volume_drag / set_volume_from_mouse /
    │             volume_from_x / volume_tooltip_label / set_alt_pressed /
    │             set_now_playing_info_hovered / set_hovered_now_playing_link /
    │             click_now_playing_link
    ├── Waveform cache: cached_waveform / invalidate_waveform_for_path / clear_waveform_cache
    └── Free functions: decode_or_load_waveform / decode_waveform / decode_waveform_sampled /
                         decode_waveform_full / normalize_waveform_peaks / generate_fallback_waveform /
                         generate_loading_waveform / waveform_loading_phase
```

## Test results

- `rtk cargo test`: **29 passed, 3 ignored** (same as pre-migration baseline)
- `rtk cargo fmt`: clean
- `rtk cargo check`: clean
- `rtk cargo build --release`: clean

## Future extensibility unlocked by this work

The `PlayerEntity` API surface intentionally includes several methods
marked `#[allow(dead_code)]` that map to imminent feature work:

- **MPRIS / D-Bus media controls**: `set_playback_volume`, `seek_playback`,
  `click_now_playing_link`, `close_output_menu` are all callable from
  external integrations without UI involvement.
- **Profile switching**: `replace_catalog` swaps the catalog handle
  while keeping the path-keyed waveform cache valid.
- **Headless rendering / inspector**: `playback_status` (long form),
  `is_volume_dragging`, `alt_pressed` are exposed for callers that
  don't have a `Window` in scope.
- **Symphonia/CPAL backend swap (per PRD)**: `PlayerEntity` already
  isolates the `PlaybackController` dependency. A swap touches only
  `entity.rs::start_deferred_init` + the audio command methods; the
  event surface stays the same.

## File diff summary

```
src/app/player.rs        → renamed to src/app/player/mod.rs + heavily refactored
src/app/player/entity.rs → new (952 LOC)
src/app/mod.rs           → 17 fields removed, 4 added; constructor wiring; event handler
src/app/library_state.rs → reset_for_library_reload + apply_library_event(cx)
                            + apply_removed_track_paths(cx); waveform cache sync removed
src/app/table.rs         → render_table threads is_playing through 3 helpers
src/app/history.rs       → render_playback_history_row threads is_playing
src/app/settings.rs      → 1-line current_output_label() → player.read(cx).current_output_label()
PHASE3_16_RESULTS.md     → this file (new)
```

## Recommendation

This work is **ready to commit as a foundational refactor**. The perf
benefit will materialize once `PlayerEntity` becomes a renderable view
(option 1 above), which is now a tractable, isolated next step rather
than the multi-day refactor it would have been against the monolithic
`TempoApp`.
