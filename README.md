# Tempo

Tempo is an early Rust/GPUI prototype for a fast, local-first music player and library manager. The goal is a dense, keyboard-friendly desktop app for large local collections, with Arch Linux as the first target and cross-platform support later.

The project is inspired more by Foobar2000's workflow and architecture than by modern streaming apps: table-first browsing, fast metadata search, reliable playback, queue control, local files, and minimal visual overhead.

## Current State

This repository currently contains a GPUI shell prototype, not a complete player. It uses mock track data to explore the core interaction model and layout before committing to the library, database, and playback layers.

Implemented prototype behavior:

- Dense single-window UI with left navigation, central table, right queue, and bottom player bar.
- Collapsible sidebars that disappear fully and expose reopen buttons in the main content area.
- Sortable table headers with visible ascending/descending indicators.
- Table row selection, double-click to play, keyboard play/pause, and selection movement.
- Right-click row context menu overlay.
- Full-height waveform-style seekbar in the player bar.
- Transport controls placed above the volume row.
- Settings screen placeholder for future preferences.

## Inspiration

Tempo borrows these ideas from Foobar2000:

- Table-centric music management instead of album-art-first browsing.
- Fast handling of large local libraries.
- Powerful metadata-oriented workflows.
- Simple, predictable playback and queue behavior.
- Broad native decoder support, with optional decoder fallbacks later.
- No account model, telemetry, recommendations, or streaming-service assumptions.

Tempo is not trying to clone Foobar2000's UI exactly. The design target is a native Linux app that preserves Foobar's speed and density while using a modern GPU-rendered Rust UI.

## Implementation

The current app is a Rust binary using `gpui = "0.2.2"`.

Current files:

- `src/main.rs`: GPUI prototype, mock data, table rendering, sidebars, player bar, settings view, keyboard actions.
- `PRD.md`: Product requirements and architecture notes.
- `Cargo.toml`: Rust package manifest.

The prototype intentionally keeps everything in `src/main.rs` while the UI direction is still changing quickly. Once the shell stabilizes, the app should be split into modules/crates so GPUI-specific code is isolated from playback, library scanning, database, and platform integration.

## Architecture Direction

The intended long-term architecture is a Rust workspace with isolated responsibilities:

| Area | Responsibility |
|---|---|
| `app` | Main binary, startup, dependency wiring. |
| `ui` | GPUI views, commands, keybindings, context menus, tray popover. |
| `core` | Track, album, playlist, queue, playback state, domain types. |
| `library` | Folder scanning, monitoring, import jobs. |
| `metadata` | Tag reading, artwork extraction, normalization. |
| `db` | SQLite schema, migrations, query APIs. |
| `search` | Fuzzy all-fields search, ranking, sorting/filtering. |
| `playback` | Decode pipeline, output, buffering, seeking, gapless playback. |
| `platform` | XDG paths, MPRIS, media keys, Linux tray/status notifier. |
| `artwork` | Thumbnail generation, cache, artwork source selection. |

Recommended stack from the current plan:

- UI: GPUI.
- Audio decoding: Symphonia first.
- Audio output: CPAL.
- Metadata: Lofty.
- Database: SQLite via `rusqlite`, with FTS5.
- Filesystem watching: `notify` with debouncing.
- Linux media integration: MPRIS over D-Bus.
- Linux tray/taskbar: `ksni` StatusNotifierItem.
- Config/cache paths: XDG Base Directory conventions.

## Design Principles

- Local-first: no network dependency for core library and playback.
- Fast by default: startup, search, sorting, and playback controls should stay responsive with very large libraries.
- Dense UI: prioritize visible tracks and metadata over spacious cards or large artwork.
- Keyboard-friendly: common playback and navigation actions should work without touching the mouse.
- Separated core: playback/library/database code should not depend on GPUI.
- Native Linux first: Arch Linux is the initial target, with portable abstractions where they do not slow development.
- No tag editing in MVP: read and index metadata, but do not write tags yet.

## Keyboard And Mouse Prototype

- Left click row: select.
- Double click row: play.
- Right click row: select and show context menu.
- `Enter`: play selected row from start.
- `Space`: toggle pause/play.
- `Left`: move table selection up.
- `Right`: move table selection down.
- Table headings: sort by clicked column, click again to reverse direction.

## TODO

- Run and visually tune the GPUI prototype with real screenshots/runtime feedback.
- Split `src/main.rs` into UI modules before it grows much further.
- Replace mock track data with domain models and persistent state.
- Implement a real text input/search component.
- Add library folder settings and XDG-backed config storage.
- Design SQLite schema and migrations for tracks, albums, artists, playlists, queue, stats, artwork, and search index.
- Implement recursive library scanning with `notify`-based monitoring.
- Add metadata extraction via `lofty`.
- Add artwork extraction, folder-art fallback, and thumbnail cache.
- Implement fuzzy all-fields search backed by SQLite FTS5 candidates.
- Build playback pipeline with Symphonia and CPAL.
- Add seeking, next/previous, queue mutation, repeat/shuffle, and volume persistence.
- Implement gapless playback where supported.
- Add MPRIS support for media keys and desktop shell controls.
- Add Linux tray/status notifier using `ksni`.
- Add settings for UI scale, sidebar state, music folders, output device, and decoder behavior.
- Add optional FFmpeg fallback after native decoding is stable.
- Add ReplayGain support later with Off, Track, Album, and Smart modes.
- Add tests for sorting, search ranking, metadata normalization, database migrations, and queue behavior.
- Add packaging for Arch Linux once the app has real playback and library scanning.

## Development

Format and check the prototype with:

```sh
rtk cargo fmt && rtk cargo check
```

Run the app with:

```sh
rtk cargo run
```
