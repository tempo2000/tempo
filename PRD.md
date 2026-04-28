# Minimal Rust/GPUI Music Player PRD

## Summary

Build a minimal, extremely fast, cross-platform local music player/library in Rust using GPUI, with Arch Linux as the first target. The product should feel closer to Foobar2000 than to modern streaming apps: table-first, metadata-first, fast search, powerful local library handling, and reliable playback.

Working codename: `tempo`.

## Vision

Create a native Linux music player for large local libraries that starts quickly, scans efficiently, searches instantly, plays reliably, and avoids unnecessary visual or network complexity.

The app should be local-first, keyboard-friendly, privacy-preserving, and small enough to feel like a tool rather than a media portal.

## Research Basis

### Foobar2000

Foobar2000's durable strengths are not visual design. They are architecture and workflow:

- Fast media library with monitored folders.
- Efficient handling of large playlists and libraries.
- Broad native decoder support plus optional decoder components.
- Advanced metadata model and query syntax.
- Table/playlist-centric UI.
- Gapless playback.
- ReplayGain support.
- Customizable keyboard shortcuts.
- Component architecture for decoders, DSPs, UI panels, search tools, tagging, statistics, and utilities.
- No telemetry or account model.

### GPUI

GPUI is a GPU-accelerated Rust UI framework from Zed. It supports declarative views, low-level custom elements, actions/keybindings, async integration with the event loop, and examples for large virtualized lists.

Important risk: GPUI is still pre-1.0 and may introduce breaking changes. This is acceptable for an early product if the app keeps UI code isolated from the library/playback core.

### Linux Desktop

Linux should follow XDG Base Directory rules for config, state, data, cache, and runtime files. Playback controls should integrate with MPRIS so desktop shells, media keys, lock screens, and external controllers can control the player.

For the requested small taskbar/tray control, Linux implementation should use a StatusNotifier/AppIndicator-compatible tray icon where available, plus MPRIS as the more standard media integration path. The tray icon can open a compact GPUI now-playing popover with current track, pause, previous, next, and random/shuffle controls.

## Product Goals

- Start fast and remain responsive with 100k+ tracks.
- Scan and monitor local music folders.
- Extract metadata tags and artwork.
- Provide simple fuzzy search across all indexed fields.
- Play common local formats with gapless support where available.
- Use a single-window, table-first UI.
- Provide a small tray/taskbar control for now-playing actions.
- Avoid tag editing in MVP.
- Defer ReplayGain scanning/application until after the core player is excellent.
- No telemetry, accounts, cloud sync, recommendations, or streaming-service assumptions.

## Non-Goals

- No tag editing in MVP.
- No CD ripping.
- No transcoding/converter pipeline.
- No plugin SDK in MVP.
- No visualizers in MVP.
- No lyrics in MVP.
- No online album-art lookup in MVP.
- No streaming-service integration.
- No Foobar-compatible title-formatting language in MVP.

## Target Users

- Arch/Linux users with large local collections.
- Foobar2000 users who currently rely on Wine.
- Users who prefer fast tables, metadata, playlists, and keyboard control.
- Users with mixed FLAC, MP3, M4A/AAC, Opus, Ogg Vorbis, WAV, AIFF, and WavPack libraries.

## Foobar2000 Decoder Model

Foobar2000 handles formats in three layers:

| Layer | Foobar2000 Behavior | Product Implication |
|---|---|---|
| Native decoders | Foobar2000 ships with broad built-in support for mainstream audio formats. | MVP should support the most common formats natively through Rust libraries. |
| Decoder components | Users can install optional components for more formats, including game music and niche codecs. | Post-MVP can add an internal extension boundary or optional format modules. |
| FFmpeg wrapper | Foobar2000 can use an FFmpeg Decoder Wrapper component on Windows, requiring user-supplied `ffmpeg.exe`/`ffprobe.exe`. macOS has built-in FFmpeg-style support for extra formats. | We should add an optional FFmpeg fallback setting after the native pipeline is stable. |
| Command-line decoder wrapper | Foobar2000 can route arbitrary formats through standalone command-line decoders. | This is powerful but should be deferred until core playback is mature. |

Recommendation: use native Rust decoders first, then add optional FFmpeg fallback behind an explicit setting.

MVP behavior:

- Prefer native decoding for supported formats.
- Mark unsupported files as unsupported during scan.
- Do not require FFmpeg for first launch.

Phase 2 behavior:

- Add setting: `Decoders: Native only`, `Native then FFmpeg fallback`, `FFmpeg first`.
- Use `ffprobe` for metadata only when native tag extraction fails and fallback is enabled.
- Use `ffmpeg` as a decode process for unsupported files when fallback is enabled.
- Clearly label tracks that require fallback decoding.
- Surface missing FFmpeg with a useful settings warning, not a startup failure.

## MVP Scope

| Area | Requirement |
|---|---|
| Library | Add/remove music folders, recursively scan, monitor changes. |
| Metadata | Read core tags, technical info, duration, and existing artwork. |
| Search | Simple fuzzy search across all fields. |
| Playback | Play/pause/seek/next/previous, queue, volume, basic gapless. |
| Artwork | Embedded art and folder art, cached thumbnails. |
| UI | Single main window with table, search bar, playback bar, optional compact details/artwork area. |
| Tray/taskbar | Small icon opens now-playing controls: track, pause, previous, next, random/shuffle. |
| Playlists | Manual playlists and persistent queue/session. |
| Linux | Arch-first packaging, MPRIS, media keys, XDG paths. |
| Privacy | No network access by default. |

## Recommended Technical Stack

| Concern | Recommendation | Rationale |
|---|---|---|
| UI | `gpui` | Fast Rust UI, virtualized lists, keyboard actions. |
| Audio decode | `symphonia` | Pure Rust decode/demux for major formats, gapless support for several codecs. |
| Audio output | `cpal` | Direct control over buffering, output devices, and gapless pipeline. |
| Prototype playback | `rodio` optional | Useful for a quick spike, but likely too high-level for final gapless behavior. |
| Metadata | `lofty` | Strong tag reading and artwork extraction support. |
| Database | SQLite via `rusqlite` with bundled SQLite | Stable local DB, easy packaging, FTS5 support. |
| Search | SQLite FTS5 plus Rust fuzzy scoring | Simple all-fields fuzzy search without committing to a complex query language. |
| Advanced search later | `tantivy` optional | Better for huge-scale fuzzy/natural search if SQLite plus Rust scoring is insufficient. |
| Filesystem watching | `notify` with debouncing | Cross-platform watcher, inotify on Linux. |
| Tray/taskbar | `ksni` on Linux, `tray-icon` only if cross-platform tray is needed | `ksni` implements StatusNotifierItem over D-Bus without GTK; better fit for a GPUI Linux-first app. |
| Linux media control | MPRIS over D-Bus | Standard desktop/player control integration. |
| Config paths | XDG Base Directory | Native Linux behavior. |

## Architecture

Use a Rust workspace with isolated crates so GPUI churn does not destabilize playback/library internals.

| Crate | Responsibility |
|---|---|
| `app` | Main binary, startup, dependency wiring. |
| `ui` | GPUI views, commands, keybindings, tray popover. |
| `core` | Track, album, playlist, playback, search domain types. |
| `library` | Scan orchestration, folder monitoring, import jobs. |
| `metadata` | Tag extraction, artwork discovery, normalization. |
| `db` | SQLite schema, migrations, query APIs. |
| `search` | All-fields fuzzy search, ranking, sort/filter logic. |
| `playback` | Decoder, queue, audio output, buffering, seeking. |
| `platform` | Linux MPRIS, XDG paths, tray/taskbar, desktop file. |
| `artwork` | Thumbnail generation/cache and artwork source selection. |

## Data Model

| Table | Purpose |
|---|---|
| `library_roots` | Configured music folders and scan status. |
| `tracks` | One row per indexed audio file. |
| `albums` | Normalized album grouping. |
| `artists` | Normalized artist records. |
| `track_artists` | Many-to-many artist mapping. |
| `playlists` | User playlists. |
| `playlist_items` | Ordered playlist entries. |
| `playback_stats` | Play count, last played, skip count. |
| `artwork` | Artwork cache records and source mapping. |
| `scan_events` | Errors, warnings, last scan metadata. |
| `tracks_fts` | Search index over title, artists, album, genre, date, codec, path, comments, and raw searchable tags. |

Initial track identity should use normalized absolute path plus file size and modification time. Later, add stable file IDs or content hashes for better rename detection.

## Functional Requirements

### Library Scanning

- Users can add one or more root folders.
- Scanner recursively discovers supported audio files.
- Scanner ignores hidden files and folders by default.
- Scanner supports include/exclude patterns.
- Scanner runs in the background without blocking UI.
- Scanner persists progress and can resume after restart.
- Scanner detects new, changed, moved, and deleted files.
- Scanner records unreadable/corrupt files without failing the whole scan.
- Scanner exposes folder states: scanning, monitoring, idle, pending, error.

### Metadata Extraction

- Extract title, artist, album, album artist, genre, date/year, track number, disc number, duration, codec, sample rate, channel count, bitrate, path, file size, and modified time.
- Extract embedded artwork when present.
- Normalize multi-value artist and genre fields.
- Normalize album identity using album artist, album, date, and disc metadata.
- Read ReplayGain tags when present, but do not apply them in MVP unless the playback layer already makes this trivial.
- Store raw tags for future display/search/debugging.
- Do not write tags in MVP.

### Artwork

- Prefer embedded front cover art.
- Fall back to folder images named `cover`, `folder`, `front`, or `album`, case-insensitive.
- Cache decoded thumbnails under `$XDG_CACHE_HOME/tempo/artwork`.
- Store artwork content hash to deduplicate cache entries.
- Generate small and medium thumbnails.
- Never block search or playback on artwork extraction.

### Search

MVP search should be simple and fuzzy.

- One search box searches all fields.
- Search fields include title, artist, album artist, album, genre, date, codec, path, filename, comments, and raw searchable tags.
- Query matching is case-insensitive and accent-insensitive where practical.
- Results rank higher when matches occur in title, artist, album, then path.
- Results rank higher for contiguous and prefix matches.
- Results update interactively while typing.
- Search should tolerate minor typos and partial words.
- Search should not expose Foobar-style query operators in MVP.

Suggested implementation:

- Maintain a compact normalized `search_blob` per track.
- Use SQLite FTS5/prefix/trigram where useful to get candidate rows.
- Apply Rust-side fuzzy scoring to candidates for final ranking.
- Keep the API clean so the query language can evolve later without rewriting UI code.

Post-MVP search:

- Add field filters like `artist radiohead`, `album kid a`, or `genre jazz` if users need them.
- Consider Foobar-like operators later: `HAS`, `IS`, `AND`, `OR`, `NOT`, `MISSING`, `PRESENT`, `SORT BY`.
- Consider Tantivy if SQLite plus Rust fuzzy scoring is not enough.

### Playback

- Play selected track immediately.
- Maintain a playback queue.
- Support play, pause, stop, next, previous, seek, and volume.
- Support random/shuffle playback mode.
- Preload enough of the next track to support gapless transitions where format metadata and decoder support allow it.
- Do not let scan/search/database work interrupt the audio callback.
- Handle decode errors by skipping with a visible warning.
- Handle audio device disconnects gracefully.

### ReplayGain

ReplayGain is deferred until after MVP playback and library UX are stable.

When implemented, expose a setting with these modes:

| Mode | Behavior |
|---|---|
| Off | Do not apply ReplayGain. |
| Track | Apply per-track gain. Best for shuffled/random playback. |
| Album | Apply album gain when available. Best for album playback. |
| Smart | Use album gain for sequential album playback and track gain for random/shuffle/queue playback. |

ReplayGain scanner support should be later than ReplayGain tag application.

### Playlists

- Users can create, rename, and delete playlists.
- Users can add selected tracks, albums, or search results to playlists.
- Users can reorder playlist rows.
- App restores the last active playlist/session.
- Smart/autoplaylists are post-MVP.

### Main UI

The UI is single-window and table-first.

Default layout:

| Region | Contents |
|---|---|
| Top | Search box and small command/status area. |
| Center | Virtualized track table. |
| Bottom | Playback bar with track title, position, play/pause, previous, next, random/shuffle, volume. |
| Optional side/inline area | Small artwork and metadata for selected or now-playing track. |

Table columns:

- Playing indicator.
- Title.
- Artist.
- Album.
- Date/year.
- Track.
- Duration.
- Codec.

MVP keyboard shortcuts:

| Shortcut | Action |
|---|---|
| `/` or `Ctrl+F` | Focus search. |
| `Enter` | Play focused track. |
| `Space` | Play/pause. |
| `J/K` or arrow keys | Move selection. |
| `Ctrl+N` | Next track. |
| `Ctrl+P` | Previous track. |
| `R` | Toggle random/shuffle. |
| `Ctrl+L` | Add library folder. |

### Tray/Taskbar Control

The app should provide a small taskbar/tray icon on Linux where the desktop supports it.

Requirements:

- Tray icon is visible while the app is running if enabled.
- Left-click opens or focuses the main window by default.
- Secondary action opens a compact now-playing popover or menu.
- Popover/menu shows current track title and artist.
- Controls include pause/play, previous, next, and random/shuffle.
- Tooltip displays currently playing track.
- Tray icon remains optional because tray support varies across Linux desktops.
- MPRIS remains the canonical desktop integration for media keys and shell media widgets.

Linux implementation notes:

- Prefer `ksni` for Linux because it implements the StatusNotifierItem protocol directly over D-Bus and avoids GTK/AppIndicator event-loop dependencies in the GPUI process.
- Keep `tray-icon` as the cross-platform fallback if/when Windows and macOS tray support become a priority.
- MPRIS is separate from tray support. MPRIS handles media keys and shell media widgets; StatusNotifier/AppIndicator handles the small panel/taskbar icon and menu.
- Wayland environments differ. KDE Plasma generally works; GNOME often requires an AppIndicator/KStatusNotifier extension; wlroots/Hyprland/Sway setups need a configured tray host such as a Waybar tray module.
- If tray support is unavailable, the app should still expose MPRIS controls.

### Linux Integration

- Store config in `$XDG_CONFIG_HOME/tempo`.
- Store persistent state/database in `$XDG_STATE_HOME/tempo` or `$XDG_DATA_HOME/tempo`.
- Store thumbnails/cache in `$XDG_CACHE_HOME/tempo`.
- Provide `.desktop` file and icons.
- Expose MPRIS player at `org.mpris.MediaPlayer2.tempo`.
- Support media keys through MPRIS/desktop integration.
- Prefer PipeWire-friendly behavior through CPAL backend selection.

## Performance Requirements

| Metric | MVP Target |
|---|---|
| First paint | Less than 500 ms on warm start. |
| Warm startup to usable search | Less than 1 second for 100k-track library. |
| Idle memory | Less than 150 MB with 100k tracks indexed. |
| Common search latency | Less than 50 ms p95 after indexing. |
| UI responsiveness | No visible stalls during scan/search/playback. |
| Scan throughput | At least 50 tracks/sec on warm local SSD for common formats. |
| Playback CPU | Less than 3 percent on a modern desktop while playing FLAC/MP3. |
| Gapless transition | No audible gap for supported formats. |

## Arch Linux Dependencies

Base development dependencies likely needed:

```sh
rustup gcc clang cmake pkgconf alsa-lib pipewire fontconfig glib2 wayland libxcb libxkbcommon-x11 sqlite openssl zstd xdg-desktop-portal git
```

Preferred Linux tray/taskbar implementation via `ksni` should only need a working D-Bus session and a desktop/panel tray host.

Cross-platform tray fallback dependencies likely needed if using `tray-icon` on Linux:

```sh
gtk3 libappindicator-gtk3
```

Alternative tray dependency:

```sh
libayatana-appindicator
```

Optional FFmpeg fallback dependency for Phase 2:

```sh
ffmpeg
```

## Milestones

| Milestone | Outcome |
|---|---|
| M0 Prototype | GPUI window, SQLite DB, one-file playback spike. |
| M1 Library Scanner | Add folder, scan files, extract tags, persist tracks. |
| M2 Fuzzy Search Table | Virtualized table, all-fields fuzzy search, simple sorting. |
| M3 Playback Core | Queue, controls, seek, next/previous, random/shuffle, basic gapless. |
| M4 Artwork | Embedded/folder artwork extraction and thumbnail cache. |
| M5 Linux Desktop | MPRIS, media keys, tray/taskbar control, XDG paths, desktop file. |
| M6 Beta Polish | Error handling, performance profiling, packaging, docs. |
| M7 Format Fallback | Optional FFmpeg metadata/decode fallback setting. |
| M8 ReplayGain | User-selectable ReplayGain mode using existing tags. |

## Risks

- GPUI is pre-1.0 and may change quickly.
- GPUI's Windows support should be validated before committing to parity timelines.
- Gapless playback requires a carefully designed decode/output pipeline.
- Linux tray/taskbar support is fragmented across desktop environments.
- SQLite FTS5 is not true fuzzy search by itself; Rust-side fuzzy ranking may be needed.
- Very large libraries may require careful memory and query planning.
- Inotify watch limits can affect huge folder trees.
- Metadata edge cases are endless and should not block core playback/search.
- Optional FFmpeg fallback introduces process management, latency, packaging, and licensing considerations.
- Symphonia's MPL-2.0 license should be reviewed before final product licensing decisions.

## Open Decisions

- Final app name.
- Exact Rust fuzzy matcher crate.
- Whether SQLite FTS5 trigram indexes are sufficient for candidate generation.
- Whether first playback implementation spikes with Rodio before moving to CPAL directly.
- Whether tray/taskbar support should be enabled by default on Linux or exposed as an optional feature.

## Recommended MVP Decision Set

- No tag editing.
- Single-window, table-first UI.
- All-fields fuzzy search only.
- Native decoder pipeline first.
- Optional FFmpeg fallback later, modeled after Foobar2000's wrapper approach.
- ReplayGain later with user-selectable mode: off, track, album, smart.
- MPRIS plus optional tray/taskbar now-playing control.
- No network access by default.
