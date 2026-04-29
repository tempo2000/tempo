//! Player module — owns the [`PlayerEntity`] and all rendering /
//! state-mutation code for the player bar (transport, waveform,
//! volume, output device picker).
//!
//! See [`entity`] for the architectural design notes. This file has
//! two purposes:
//!
//! 1. **Render the player bar** — transport, waveform seekbar,
//!    Now-Playing strip, volume, output picker. Rendering reads from
//!    `self.player.read(cx)` and binds clicks/hovers via
//!    `self.player.update(cx, |p, cx| p.foo(...))`. The render code
//!    still lives on `TempoApp` (not `PlayerEntity::render`) because
//!    it needs the active track's metadata from `self.tracks` to
//!    paint the title/artist/album marquees and the album tile.
//!
//!    Deferring the move of `render_player_bar` onto `PlayerEntity`
//!    itself avoids duplicating the `Track` lookup logic; once the
//!    `PlayerEntity` carries enough metadata (via
//!    [`PlayerEvent::PlayingTrackChanged`] payload extension or a
//!    push from `TempoApp` after `play_track`), this can move
//!    wholesale.
//!
//! 2. **TempoApp glue methods** — `play_track`, `play_finished_track`,
//!    `play_random_track`, `play_adjacent_track`, `queue_*`,
//!    `seek_playback_to`, etc. These do cross-region bookkeeping
//!    (play counts, history, queue, tabs) and then delegate the
//!    audio-side work to `PlayerEntity` via `self.player.update`.

use super::*;
use std::time::Instant;

mod entity;

pub(super) use entity::{PlayerEntity, PlayerEvent};

// ============================================================================
// TempoApp glue methods — orchestration that touches both player state
// (audio backend) and parent state (tracks, queue, history, tabs).
// ============================================================================

impl TempoApp {
    /// Single subscriber callback wired in `TempoApp::new`. Translates
    /// each [`PlayerEvent`] into the right cross-region update on
    /// `TempoApp`. Keeping this one method keeps the event vocabulary
    /// visible in one place; new variants get a new arm here.
    pub(super) fn handle_player_event(&mut self, event: &PlayerEvent, cx: &mut Context<Self>) {
        match event {
            PlayerEvent::PlayingTrackChanged { path } => {
                if let Some(path) = path {
                    if let Some(&ix) = self.track_path_index.get(path) {
                        self.playing_track = ix;
                    }
                }
                cx.notify();
            }
            PlayerEvent::IsPlayingChanged(is_playing) => {
                // Table active-row transport icon (Ⅱ vs ▶) and the
                // play-count column need to repaint.
                perf::event(
                    "player.is_playing_changed",
                    format!("is_playing={is_playing}"),
                );
                cx.notify();
            }
            PlayerEvent::TrackFinished { finished_path } => {
                let is_finished_path = self
                    .player
                    .read(cx)
                    .playing_track_path()
                    .is_some_and(|current| current == finished_path.as_path());
                if !is_finished_path {
                    // The user already moved on (manual skip) — ignore
                    // the stale auto-advance.
                    return;
                }
                self.play_finished_track(cx);
            }
            PlayerEvent::NowPlayingLinkClicked { kind, path } => {
                let Some(&track_ix) = self.track_path_index.get(path) else {
                    // Track was removed from library while it kept
                    // playing — silently drop the click.
                    return;
                };
                match kind {
                    NowPlayingLink::Title => self.select_track_in_all_music(track_ix),
                    NowPlayingLink::Artist => self.open_artist_tab_for_track(track_ix),
                    NowPlayingLink::Album => self.open_album_tab_for_track(track_ix),
                }
                cx.notify();
            }
            PlayerEvent::StateMutated => {
                self.refresh_player_state_snapshot(cx);
                self.save_app_state();
            }
        }
    }

    /// Refresh the denormalized `volume_snapshot` and
    /// `output_device_snapshot` mirrors from authoritative
    /// [`PlayerEntity`] state. Called after every
    /// [`PlayerEvent::StateMutated`] and once more at shutdown via the
    /// `on_app_quit` hook so the final save is consistent.
    pub(super) fn refresh_player_state_snapshot(&mut self, cx: &gpui::App) {
        let player = self.player.read(cx);
        self.volume_snapshot = player.volume();
        self.output_device_snapshot = player.output_device().map(str::to_string);
    }

    pub(super) fn start_deferred_playback_init(&self, cx: &mut Context<Self>) {
        self.player.update(cx, |player, player_cx| {
            player.start_deferred_init(player_cx)
        });
    }

    pub(super) fn start_playback_tick(&self, cx: &mut Context<Self>) {
        self.player.update(cx, |player, player_cx| {
            player.start_playback_tick(player_cx)
        });
    }

    pub(super) fn remove_track_from_queue(&mut self, removed_ix: usize) {
        self.queue = self
            .queue
            .iter()
            .filter_map(|track_ix| {
                if *track_ix == removed_ix {
                    None
                } else if *track_ix > removed_ix {
                    Some(*track_ix - 1)
                } else {
                    Some(*track_ix)
                }
            })
            .collect();
    }

    pub(super) fn queue_track(&mut self, track_ix: usize) {
        self.queue_track_at_end(track_ix);
    }

    pub(super) fn queue_track_at_start(&mut self, track_ix: usize) {
        if track_ix >= self.tracks.len() {
            return;
        }

        self.queue.insert(0, track_ix);
        self.right_sidebar_collapsed = false;
        self.context_menu_track = None;
    }

    pub(super) fn queue_track_at_end(&mut self, track_ix: usize) {
        if track_ix >= self.tracks.len() {
            return;
        }

        self.queue.push(track_ix);
        self.right_sidebar_collapsed = false;
        self.context_menu_track = None;
    }

    pub(super) fn queue_album_from_track(&mut self, track_ix: usize, shuffled: bool) {
        let Some(album) = self.tracks.get(track_ix).map(|track| track.album.clone()) else {
            return;
        };

        let mut album_tracks = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(ix, track)| (track.album == album).then_some(ix))
            .collect::<Vec<_>>();

        if shuffled {
            let seed = Self::shuffle_seed();
            album_tracks.sort_by_key(|track_ix| {
                Self::shuffle_key(&self.tracks[*track_ix], *track_ix, seed)
            });
        }

        self.queue.extend(album_tracks);
        self.right_sidebar_collapsed = false;
        self.context_menu_track = None;
    }

    pub(super) fn shuffle_seed() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or_default()
    }

    pub(super) fn shuffle_key(track: &Track, track_ix: usize, seed: u64) -> u64 {
        let mut hash = seed ^ ((track_ix as u64).wrapping_mul(0x9e3779b97f4a7c15));

        for part in [&track.title, &track.artist, &track.album] {
            for byte in part.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }

        hash ^ (hash >> 33)
    }

    pub(super) fn clamp_track_indices(&mut self, cx: &mut Context<Self>) {
        if self.tracks.is_empty() {
            for tab in &mut self.tabs {
                tab.selected_track = 0;
            }
            self.playing_track = 0;
            self.context_menu_track = None;
            // The previous monolithic implementation merely flipped
            // `is_playing = false` without stopping the audio
            // backend; calling `player.stop()` here also drains the
            // rodio sink, so a subsequent `restart_library_watcher`
            // doesn't leave half-decoded audio queued. This matches
            // user expectations ("library is empty, so should be
            // silent").
            self.player.update(cx, |player, cx| player.stop(cx));
            return;
        }

        let last = self.tracks.len() - 1;
        self.playing_track = self.playing_track.min(last);

        for tab_ix in 0..self.tabs.len() {
            let selected_track = self.tabs[tab_ix].selected_track.min(last);
            let replacement_track = {
                let indices = self.track_indices_for_tab(tab_ix);
                (!indices.contains(&selected_track)).then(|| indices.first().copied().unwrap_or(0))
            };
            let tab = &mut self.tabs[tab_ix];
            tab.selected_track = replacement_track.unwrap_or(selected_track);
        }

        if self
            .context_menu_track
            .is_some_and(|track_ix| track_ix > last)
        {
            self.context_menu_track = None;
        }

        self.queue.retain(|track_ix| *track_ix <= last);
    }

    pub(super) fn play_track(&mut self, track_ix: usize, cx: &mut Context<Self>) {
        self.play_track_with_history(track_ix, true, cx);
    }

    pub(super) fn play_track_with_history(
        &mut self,
        track_ix: usize,
        record_history: bool,
        cx: &mut Context<Self>,
    ) {
        let start = Instant::now();
        let Some(track) = self.tracks.get(track_ix) else {
            return;
        };
        let track_path = track.path.clone();

        self.playing_track = track_ix;
        self.set_active_selected_track(track_ix);
        self.context_menu_track = None;

        let result = self.player.update(cx, |player, cx| {
            player.start_playback(track_path.clone(), cx)
        });

        if result.is_ok() {
            // Increment play count via catalog (authoritative across
            // restarts) with an in-memory fallback. Then mirror the
            // count onto the in-memory `Track` so the table column
            // reflects it without a query round-trip.
            let plays = perf::time(
                "player.increment_play_count",
                format!("path={}", track_path.display()),
                || {
                    self.catalog
                        .as_ref()
                        .and_then(|catalog| catalog.increment_play_count(&track_path).ok())
                        .unwrap_or_else(|| self.tracks[track_ix].plays.saturating_add(1))
                },
            );
            if let Some(track) = self.tracks.get_mut(track_ix) {
                track.plays = plays;
            }
            if record_history {
                self.record_playback_history(track_ix);
            }
        }
        perf::log_duration(
            "player.play_track",
            start.elapsed(),
            format!(
                "track_ix={track_ix} record_history={record_history} path={}",
                track_path.display()
            ),
        );
    }

    /// Smart play/pause: if currently playing, pause; if paused but
    /// the backend has a loaded track, resume; if the backend is
    /// empty, restart from `self.playing_track`. This lives on
    /// `TempoApp` because the "restart from index" case needs the
    /// `tracks` list.
    pub(super) fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.tracks.is_empty() {
            return;
        }

        let (is_playing, resumable) = {
            let player = self.player.read(cx);
            let is_playing = player.is_playing();
            (is_playing, !is_playing && player.has_playback())
        };

        if is_playing {
            self.player.update(cx, |player, cx| player.pause(cx));
            self.context_menu_track = None;
            return;
        }

        if resumable {
            let resumed = self.player.update(cx, |player, cx| player.resume(cx));
            if !resumed {
                // Backend was empty; fall through to a fresh
                // start_playback below.
                self.play_track(self.playing_track, cx);
            }
            self.context_menu_track = None;
            return;
        }

        // No backend yet (still initializing) — kick off start_playback
        // anyway so once the backend appears, the click takes effect.
        self.play_track(self.playing_track, cx);
        self.context_menu_track = None;
    }

    pub(super) fn select_output_device(&mut self, output_name: String, cx: &mut Context<Self>) {
        let was_playing_result = self.player.update(cx, |player, cx| {
            player.select_output_device(output_name, cx)
        });
        if let Ok(true) = was_playing_result {
            // Audio backend was reset; replay the same track from
            // scratch so the new device takes over without dropping
            // the user's place in the queue.
            self.play_track_with_history(self.playing_track, false, cx);
        }
    }

    /// Programmatic volume change — currently unused by UI (the volume
    /// bar drives the player directly via `begin_volume_drag` /
    /// `drag_volume`), but exposed for future media-key / DBus
    /// integration.
    #[allow(dead_code)]
    pub(super) fn set_playback_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.player
            .update(cx, |player, cx| player.set_volume(volume, cx));
    }

    pub(super) fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.player.update(cx, |player, cx| player.toggle_mute(cx));
    }

    pub(super) fn set_max_volume(&mut self, cx: &mut Context<Self>) {
        self.player
            .update(cx, |player, cx| player.set_max_volume(cx));
    }

    pub(super) fn begin_volume_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let position = self
            .player
            .update(cx, |player, cx| player.begin_volume_drag(event, cx));
        let label = self.player.read(cx).volume_tooltip_label();
        self.show_tooltip_now("volume-tooltip", label, position, cx);
        cx.stop_propagation();
    }

    pub(super) fn drag_volume(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let drag_position = self
            .player
            .update(cx, |player, cx| player.drag_volume(event, cx));
        let Some(position) = drag_position else {
            return false;
        };
        let label = self.player.read(cx).volume_tooltip_label();
        self.show_tooltip_now("volume-tooltip", label, position, cx);
        true
    }

    pub(super) fn finish_volume_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let was_dragging = self
            .player
            .update(cx, |player, cx| player.finish_volume_drag(cx));
        if was_dragging {
            self.hide_tooltip_now("volume-tooltip", cx);
        }
        was_dragging
    }

    pub(super) fn play_adjacent_track(&mut self, delta: isize, cx: &mut Context<Self>) {
        let indices = self.current_track_indices();
        if indices.is_empty() {
            return;
        }

        let position = indices
            .iter()
            .position(|ix| *ix == self.playing_track)
            .unwrap_or(0);
        let next = (position as isize + delta).clamp(0, indices.len().saturating_sub(1) as isize);
        self.play_track(indices[next as usize], cx);
    }

    /// Auto-advance: pick the next track per the active playback
    /// mode, or stop at end-of-list for `Straight`. Called from the
    /// `PlayerEvent::TrackFinished` arm of `handle_player_event`.
    pub(super) fn play_finished_track(&mut self, cx: &mut Context<Self>) {
        let mode = self.player.read(cx).playback_mode();
        match mode {
            PlaybackMode::Loop => self.play_track(self.playing_track, cx),
            PlaybackMode::Shuffle => self.play_random_track(cx),
            PlaybackMode::Straight => {
                if let Some(next) = self.next_track_after(self.playing_track) {
                    self.play_track(next, cx);
                } else {
                    // End-of-list: surface a status string but keep
                    // the now-playing strip in place so the user can
                    // still hit play again.
                    self.player.update(cx, |player, cx| {
                        player.stop(cx);
                        player.playback_status = "Playback finished".to_string();
                    });
                }
            }
        }
    }

    pub(super) fn next_track_after(&self, track_ix: usize) -> Option<usize> {
        let indices = self.current_track_indices();
        let position = indices.iter().position(|ix| *ix == track_ix)?;
        indices.get(position + 1).copied()
    }

    pub(super) fn play_random_track(&mut self, cx: &mut Context<Self>) {
        let indices = self.current_track_indices();
        if indices.is_empty() {
            return;
        }

        let seed = Self::shuffle_seed();
        let next = indices
            .iter()
            .copied()
            .filter(|track_ix| indices.len() == 1 || *track_ix != self.playing_track)
            .min_by_key(|track_ix| Self::shuffle_key(&self.tracks[*track_ix], *track_ix, seed))
            .unwrap_or(self.playing_track);
        self.play_track(next, cx);
    }

    pub(super) fn cycle_playback_mode(&mut self, cx: &mut Context<Self>) {
        self.player
            .update(cx, |player, cx| player.cycle_playback_mode(cx));
    }

    /// Seek within the currently-playing track. If the audio backend
    /// is empty (e.g. after a stop), restarts playback from the
    /// current track first. Lives on `TempoApp` because the restart
    /// case needs the track index → path resolution.
    ///
    /// Currently exposed for media-key / scrubber integrations; the
    /// in-app waveform click goes through `seek_from_waveform_click`.
    #[allow(dead_code)]
    pub(super) fn seek_playback(&mut self, position: Duration, cx: &mut Context<Self>) {
        let needs_restart = self.player.update(cx, |player, cx| {
            // Check before calling seek so we don't burn the seek
            // request on an empty backend.
            if !player.has_playback() {
                player.playback_status = "Playback unavailable".to_string();
                return false;
            }
            !player.seek(position, cx)
        });
        if needs_restart {
            self.play_track_with_history(self.playing_track, false, cx);
            // After restart, retry the seek so the click feels
            // immediate.
            self.player
                .update(cx, |player, cx| _ = player.seek(position, cx));
        }
    }

    pub(super) fn seek_from_waveform_click(
        &mut self,
        click_x: f32,
        viewport_width: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(track) = self.tracks.get(self.playing_track) else {
            return;
        };
        let duration = track.duration_value;
        let outcome = self.player.update(cx, |player, cx| {
            player.seek_from_waveform_click(click_x, viewport_width, duration, cx)
        });
        if outcome.needs_restart {
            // Backend was empty (e.g. seek after natural end-of-track
            // stop). Restart playback for the current track and
            // re-issue the seek so the click feels immediate.
            self.play_track_with_history(self.playing_track, false, cx);
            self.player
                .update(cx, |player, cx| _ = player.seek(outcome.target, cx));
        }
    }
}

// ============================================================================
// Marquee helper — stateless, free function. Rendered by the player
// bar but kept here (rather than in a separate module) to share the
// gap/duration constants with the bar layout.
// ============================================================================

impl TempoApp {
    fn render_marquee_text(
        text: SharedString,
        animation_id: impl Into<SharedString>,
        available_width: f32,
        average_char_width: f32,
        color: u32,
    ) -> AnyElement {
        let text_width = (text.chars().count() as f32 * average_char_width).max(1.0);

        if text_width <= available_width {
            return div()
                .w_full()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(rgb(color))
                .child(text)
                .into_any_element();
        }

        let gap = 44.0;
        let scroll_distance = text_width + gap;
        let duration = Duration::from_millis(((scroll_distance / 18.0).max(7.0) * 1000.0) as u64);

        div()
            .w_full()
            .overflow_hidden()
            .text_color(rgb(color))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(div().w(px(text_width)).flex_none().child(text.clone()))
                    .child(div().w(px(gap)).flex_none())
                    .child(div().w(px(text_width)).flex_none().child(text))
                    .with_animation(
                        animation_id.into(),
                        Animation::new(duration).repeat(),
                        move |this, delta| this.ml(px(-scroll_distance * delta)),
                    ),
            )
            .into_any_element()
    }
}

// ============================================================================
// Player bar rendering — reads current state from `self.player`,
// dispatches mutations back through `self.player.update(cx, ...)`.
//
// Kept on `TempoApp` (not `PlayerEntity::render`) because it needs
// the active track's metadata from `self.tracks` to paint the
// marquees and album tile. A future refactor that pushes the active
// `Track` clone onto the player after `play_track` could move this
// wholesale; the rendering API is otherwise self-contained.
// ============================================================================

impl TempoApp {
    pub(super) fn render_player_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = *self.colors();

        if self.tracks.is_empty() {
            return div()
                .h(px(86.0))
                .flex_none()
                .flex()
                .items_center()
                .gap_4()
                .px_4()
                .border_t_1()
                .border_color(rgb(colors.button_hover))
                .bg(rgb(colors.player))
                .child(
                    div()
                        .w(px(54.0))
                        .h(px(54.0))
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(colors.border_strong))
                        .bg(rgb(colors.playing))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(colors.text_faint))
                        .child("♪"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(colors.text_strong))
                                .child(if self.is_scanning {
                                    "Scanning library"
                                } else {
                                    "Library scanner idle"
                                }),
                        )
                        .child(
                            div()
                                .text_color(rgb(colors.text_muted))
                                .child(self.visible_scan_status()),
                        ),
                )
                .into_any_element();
        }

        self.playing_track = self.playing_track.min(self.tracks.len() - 1);
        let playing_track_ix = self.playing_track;
        let source = WaveformSource::from_track(&self.tracks[playing_track_ix]);
        let (waveform, waveform_loading) = self
            .player
            .update(cx, |player, cx| player.cached_waveform(&source, cx));
        if waveform_loading {
            window.request_animation_frame();
        }
        // Snapshot all the player fields the render path consumes in
        // one pass, releasing the borrow before we touch
        // `self.tracks` / `self.colors()` / `self.album_tile_*` below.
        // `alt_pressed` lives on the entity for future use (e.g.
        // headless rendering), but the render path queries
        // `window.modifiers().alt` directly because that's the
        // authoritative source at paint time and accounts for
        // modifier presses since the last `on_modifiers_changed`
        // event.
        let (
            is_playing,
            playback_position,
            now_playing_info_hovered,
            hovered_link,
            playback_status_label,
            volume,
            output_menu_open_in_player,
        ) = {
            let player_state = self.player.read(cx);
            (
                player_state.is_playing(),
                player_state.playback_position(),
                player_state.now_playing_info_hovered(),
                player_state.hovered_now_playing_link(),
                player_state.playback_status_label(),
                player_state.volume(),
                player_state.output_menu_source() == Some(OutputMenuSource::Player),
            )
        };

        let track = &self.tracks[playing_track_ix];
        let playback_position = playback_position.min(track.duration_value);
        let playback_progress = if track.duration_value.is_zero() {
            0.0
        } else {
            (playback_position.as_secs_f32() / track.duration_value.as_secs_f32()).clamp(0.0, 1.0)
        };
        let now_playing_active_color = colors.accent;
        let show_alternate_now_playing_info = now_playing_info_hovered && window.modifiers().alt;
        let year_label = if track.year.eq_ignore_ascii_case("unknown year") {
            "Unknown Year".to_string()
        } else {
            track.year.to_string()
        };
        let alternate_status = format!("{} | {}", year_label, playback_status_label);
        let title_color = if hovered_link == Some(NowPlayingLink::Title) {
            now_playing_active_color
        } else {
            colors.text_strong
        };
        let artist_color = if hovered_link == Some(NowPlayingLink::Artist) {
            now_playing_active_color
        } else {
            colors.text_muted
        };
        let album_color = if hovered_link == Some(NowPlayingLink::Album) {
            now_playing_active_color
        } else {
            colors.text_faint
        };
        let volume_fill = PLAYER_VOLUME_BAR_W * volume;

        div()
            .id("player-bar")
            .relative()
            .h(px(86.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .border_t_1()
            .border_color(rgb(colors.button_hover))
            .bg(rgb(colors.player))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                let needs_repaint = this
                    .player
                    .update(cx, |player, _| player.set_alt_pressed(event.modifiers.alt));
                if needs_repaint {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id("now-playing-album-link")
                    .cursor_pointer()
                    .child(self.album_tile_with_hover_border(
                        track,
                        54.0,
                        Some(now_playing_active_color),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_album_tab_for_track(playing_track_ix);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("now-playing-info")
                    .w(px(220.0))
                    .flex_none()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(2.0))
                    .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                        let alt = window.modifiers().alt;
                        this.player.update(cx, |player, _| {
                            player.set_now_playing_info_hovered(*hovered, alt);
                        });
                        cx.notify();
                    }))
                    .child(if show_alternate_now_playing_info {
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(colors.text_strong))
                                    .child(Self::render_marquee_text(
                                        track.codec.clone(),
                                        SharedString::from(format!(
                                            "now-playing-codec-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        8.6,
                                        colors.text_strong,
                                    )),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(rgb(colors.text_muted))
                                    .child(Self::render_marquee_text(
                                        SharedString::from(Self::bitrate_label(track)),
                                        SharedString::from(format!(
                                            "now-playing-bitrate-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        7.8,
                                        colors.text_muted,
                                    )),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(rgb(colors.text_faint))
                                    .child(Self::render_marquee_text(
                                        SharedString::from(alternate_status),
                                        SharedString::from(format!(
                                            "now-playing-status-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        7.8,
                                        colors.text_faint,
                                    )),
                            )
                    } else {
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .id("now-playing-title-link")
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(title_color))
                                    .cursor_pointer()
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.player.update(cx, |player, _| {
                                            if *hovered {
                                                player.set_hovered_now_playing_link(Some(
                                                    NowPlayingLink::Title,
                                                ));
                                            } else if player.hovered_now_playing_link()
                                                == Some(NowPlayingLink::Title)
                                            {
                                                player.set_hovered_now_playing_link(None);
                                            }
                                        });
                                        cx.notify();
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_track_in_all_music(playing_track_ix);
                                        cx.notify();
                                    }))
                                    .child(Self::render_marquee_text(
                                        track.title.clone(),
                                        SharedString::from(format!(
                                            "now-playing-title-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        8.6,
                                        title_color,
                                    )),
                            )
                            .child(
                                div()
                                    .id("now-playing-artist-link")
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(rgb(artist_color))
                                    .cursor_pointer()
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.player.update(cx, |player, _| {
                                            if *hovered {
                                                player.set_hovered_now_playing_link(Some(
                                                    NowPlayingLink::Artist,
                                                ));
                                            } else if player.hovered_now_playing_link()
                                                == Some(NowPlayingLink::Artist)
                                            {
                                                player.set_hovered_now_playing_link(None);
                                            }
                                        });
                                        cx.notify();
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_artist_tab_for_track(playing_track_ix);
                                        cx.notify();
                                    }))
                                    .child(Self::render_marquee_text(
                                        track.artist.clone(),
                                        SharedString::from(format!(
                                            "now-playing-artist-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        7.8,
                                        artist_color,
                                    )),
                            )
                            .child(
                                div()
                                    .id("now-playing-album-text-link")
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_color(rgb(album_color))
                                    .cursor_pointer()
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.player.update(cx, |player, _| {
                                            if *hovered {
                                                player.set_hovered_now_playing_link(Some(
                                                    NowPlayingLink::Album,
                                                ));
                                            } else if player.hovered_now_playing_link()
                                                == Some(NowPlayingLink::Album)
                                            {
                                                player.set_hovered_now_playing_link(None);
                                            }
                                        });
                                        cx.notify();
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_album_tab_for_track(playing_track_ix);
                                        cx.notify();
                                    }))
                                    .child(Self::render_marquee_text(
                                        track.album.clone(),
                                        SharedString::from(format!(
                                            "now-playing-album-marquee-{playing_track_ix}"
                                        )),
                                        220.0,
                                        7.8,
                                        album_color,
                                    )),
                            )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .relative()
                    .child(self.waveform_seekbar(
                        SharedString::from(format_duration(playback_position)),
                        track.duration.clone(),
                        playback_progress,
                        waveform,
                        waveform_loading,
                        cx,
                    )),
            )
            .child(
                div()
                    .w(px(170.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_color(rgb(colors.text_muted))
                    .child(self.transport_overlay(is_playing, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("volume-mute")
                                    .cursor_pointer()
                                    .active(|this| this.opacity(0.75))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_mute(cx);
                                        cx.notify();
                                    }))
                                    .child(Self::volume_speaker_icon(1, colors)),
                            )
                            .child({
                                let volume_handle =
                                    self.player.read(cx).volume_bar_scroll_handle.clone();
                                div()
                                    .id("volume-bar")
                                    .w(px(PLAYER_VOLUME_BAR_W))
                                    .h(px(18.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .track_scroll(&volume_handle)
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                                            this.begin_volume_drag(event, cx);
                                        }),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(3.0))
                                            .rounded_full()
                                            .bg(rgb(colors.text_faint))
                                            .child(
                                                div()
                                                    .w(px(volume_fill))
                                                    .h(px(3.0))
                                                    .rounded_full()
                                                    .bg(rgb(colors.text)),
                                            ),
                                    )
                            })
                            .child(
                                div()
                                    .id("volume-max")
                                    .cursor_pointer()
                                    .active(|this| this.opacity(0.75))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_max_volume(cx);
                                        cx.notify();
                                    }))
                                    .child(Self::volume_speaker_icon(3, colors)),
                            ),
                    ),
            )
            .when(output_menu_open_in_player, |this| {
                this.child(self.player_output_device_menu(cx))
            })
            .into_any_element()
    }

    pub(super) fn playback_mode_icon(&self, cx: &gpui::App) -> &'static str {
        match self.player.read(cx).playback_mode() {
            PlaybackMode::Straight => "→",
            PlaybackMode::Loop => "↻",
            PlaybackMode::Shuffle => "⤨",
        }
    }

    pub(super) fn volume_speaker_icon(waves: usize, colors: ThemeColors) -> AnyElement {
        let color = format!("#{:06x}", colors.text_muted);
        let mut wave_paths = String::new();

        if waves >= 1 {
            wave_paths.push_str(&format!(
                r#"<path d="M14.5 9.4C15.2 10.1 15.6 11 15.6 12C15.6 13 15.2 13.9 14.5 14.6" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round"/>"#
            ));
        }

        if waves >= 2 {
            wave_paths.push_str(&format!(
                r#"<path d="M17 7.2C18.2 8.5 18.9 10.2 18.9 12C18.9 13.8 18.2 15.5 17 16.8" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round"/>"#
            ));
        }

        if waves >= 3 {
            wave_paths.push_str(&format!(
                r#"<path d="M19.4 5C21 7 22 9.4 22 12C22 14.6 21 17 19.4 19" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round"/>"#
            ));
        }

        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M3 9V15H7L12 19V5L7 9H3Z" fill="{color}"/>{wave_paths}</svg>"#
        );

        img(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            svg.into_bytes(),
        )))
        .w(px(18.0))
        .h(px(18.0))
        .into_any_element()
    }

    pub(super) fn playback_status_dropdown(
        &self,
        source: OutputMenuSource,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = *self.colors();
        let button_label = {
            let player = self.player.read(cx);
            match source {
                OutputMenuSource::Player => format!("{} ▾", player.playback_status_label()),
                OutputMenuSource::Settings => format!("{} ▾", player.current_output_label()),
            }
        };

        div()
            .id(SharedString::from(match source {
                OutputMenuSource::Player => "player-output-dropdown",
                OutputMenuSource::Settings => "settings-output-dropdown",
            }))
            .relative()
            .child(
                div()
                    .id(SharedString::from(match source {
                        OutputMenuSource::Player => "player-output-dropdown-button",
                        OutputMenuSource::Settings => "settings-output-dropdown-button",
                    }))
                    .cursor_pointer()
                    .rounded_sm()
                    .px_1()
                    .text_color(rgb(colors.text_muted))
                    .hover(move |this| this.text_color(rgb(colors.accent)).bg(rgb(colors.hover)))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        let position = event.position();
                        this.player.update(cx, |player, cx| {
                            player.toggle_output_menu(source, position, cx);
                        });
                    }))
                    .child(button_label),
            )
    }

    pub(super) fn player_output_device_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let position = self.player.read(cx).output_menu_position();
        self.menu_at(
            position,
            Corner::BottomLeft,
            point(px(0.0), px(-8.0)),
            self.output_device_menu(OutputMenuSource::Player, cx),
        )
    }

    pub(super) fn settings_output_device_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let position = self.player.read(cx).output_menu_position();
        self.menu_at(
            position,
            Corner::TopRight,
            point(px(24.0), px(18.0)),
            self.output_device_menu(OutputMenuSource::Settings, cx),
        )
    }

    pub(super) fn output_device_menu(
        &self,
        _source: OutputMenuSource,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = *self.colors();
        let current_output = self.player.read(cx).current_output_label();
        let devices = perf::time(
            "player.output_devices_for_menu",
            "",
            PlaybackController::output_devices,
        );

        self.menu_panel(260.0)
            .child(self.menu_header_with_subtitle("Audio Output", current_output.clone()))
            .when(devices.is_empty(), |this| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(colors.text_muted))
                        .child("No output devices found"),
                )
            })
            .children(
                devices
                    .into_iter()
                    .enumerate()
                    .map(move |(device_ix, device)| {
                        let selected = device.name == current_output;
                        let label = if device.is_default {
                            format!("{} (default)", device.name)
                        } else {
                            device.name.clone()
                        };
                        let output_name = device.name;

                        self.menu_item_base(SharedString::from(format!(
                            "output-device-{device_ix}"
                        )))
                        .h(px(30.0))
                        .justify_between()
                        .text_color(rgb(if selected {
                            colors.accent_soft
                        } else {
                            colors.text
                        }))
                        .hover(move |this| {
                            this.bg(rgb(colors.button_hover))
                                .text_color(rgb(colors.text_strong))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_output_device(output_name.clone(), cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        )
                        .child(if selected { "✓" } else { "" })
                    }),
            )
    }

    pub(super) fn bitrate_label(track: &Track) -> String {
        track
            .bitrate
            .map(|bitrate| format!("{bitrate} kbps"))
            .unwrap_or_else(|| "unknown bitrate".to_string())
    }

    pub(super) fn waveform_seekbar(
        &self,
        elapsed: SharedString,
        duration: SharedString,
        progress: f32,
        waveform: Arc<[f32]>,
        loading: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let progress_segments = (waveform.len() as f32 * progress).round() as usize;
        let colors = *self.colors();

        div()
            .id("waveform-seekbar")
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .cursor_pointer()
            .rounded_lg()
            .overflow_hidden()
            .bg(rgb(colors.waveform_bg))
            .border_1()
            .border_color(rgb(colors.waveform_border))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.standard_click() {
                    let click_x = f32::from(event.position().x);
                    let viewport_width = f32::from(window.viewport_size().width);
                    this.seek_from_waveform_click(click_x, viewport_width, cx);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .absolute()
                    .top(px(42.0))
                    .left_0()
                    .right_0()
                    .h(px(1.0))
                    .bg(rgb(colors.waveform_line)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(1.0))
                    // `Arc<[f32]>::iter()` borrows the slice — no clone
                    // of the underlying buffer per frame, unlike the
                    // prior `Vec<f32>` which was cloned per render.
                    .children(
                        waveform
                            .iter()
                            .copied()
                            .enumerate()
                            .map(move |(ix, height)| {
                                Self::waveform_bar(ix, height, progress_segments, loading, colors)
                            }),
                    ),
            )
            .when(loading, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_2()
                        .left_3()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(colors.waveform_bg))
                        .text_xs()
                        .text_color(rgb(colors.waveform_played_peak))
                        .child("Loading waveform"),
                )
            })
            .child(
                div()
                    .absolute()
                    .bottom_2()
                    .left_3()
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(colors.waveform_bg))
                    .text_xs()
                    .text_color(rgb(colors.text_faint))
                    .child(elapsed),
            )
            .child(
                div()
                    .absolute()
                    .bottom_2()
                    .right_3()
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(colors.waveform_bg))
                    .text_xs()
                    .text_color(rgb(colors.text_faint))
                    .child(duration),
            )
    }

    pub(super) fn waveform_bar(
        ix: usize,
        height: f32,
        progress_segments: usize,
        loading: bool,
        colors: ThemeColors,
    ) -> impl IntoElement {
        let played = ix < progress_segments;
        let playhead = ix == progress_segments;
        let peak = height > 44.0;
        let color = if loading && peak {
            colors.waveform_played
        } else if loading {
            colors.waveform_idle_peak
        } else if playhead {
            colors.waveform_playhead
        } else if played && peak {
            colors.waveform_played_peak
        } else if played {
            colors.waveform_played
        } else if peak {
            colors.waveform_idle_peak
        } else {
            colors.waveform_idle
        };

        div()
            .flex_1()
            .min_w(px(1.0))
            .h(px(if playhead { 58.0 } else { height }))
            .rounded_full()
            .bg(rgb(color))
            .opacity(if loading || played || playhead {
                1.0
            } else {
                0.78
            })
    }

    pub(super) fn transport_overlay(
        &self,
        is_playing: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let colors = *self.colors();
        let mode_icon = self.playback_mode_icon(cx);
        let mode_active = self.player.read(cx).playback_mode() != PlaybackMode::Straight;

        div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_full()
            .bg(rgb(colors.app))
            .border_1()
            .border_color(rgb(colors.waveform_border))
            .child(
                self.transport_button(mode_icon, false, mode_active)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cycle_playback_mode(cx);
                        cx.notify();
                    })),
            )
            .child(
                self.transport_button("◀", false, false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.play_adjacent_track(-1, cx);
                        cx.notify();
                    })),
            )
            .child(
                self.transport_button(if is_playing { "Ⅱ" } else { "▶" }, true, false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_playback(cx);
                        cx.notify();
                    })),
            )
            .child(
                self.transport_button("▶", false, false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.play_adjacent_track(1, cx);
                        cx.notify();
                    })),
            )
            .child(
                self.transport_button("↻", false, false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.play_random_track(cx);
                        cx.notify();
                    })),
            )
    }

    pub(super) fn transport_button(
        &self,
        label: &'static str,
        primary: bool,
        active: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let size = if primary { 28.0 } else { 22.0 };
        let hover_size = if primary { 32.0 } else { 26.0 };
        let colors = *self.colors();
        let bg = if primary {
            colors.transport_primary_bg
        } else if active {
            colors.text_strong
        } else {
            colors.player
        };
        let fg = if primary {
            colors.transport_primary_fg
        } else if active {
            colors.app
        } else {
            colors.text_muted
        };

        div()
            .id(SharedString::from(format!("transport-{label}-{primary}")))
            .w(px(size))
            .h(px(size))
            .rounded_full()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .font_weight(gpui::FontWeight::BOLD)
            .hover(move |this| {
                this.w(px(hover_size))
                    .h(px(hover_size))
                    .bg(rgb(colors.text_strong))
                    .text_color(rgb(colors.app))
            })
            .child(label)
    }
}
