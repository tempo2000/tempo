use gpui::{
    AnyElement, App, Application, Bounds, ClickEvent, Context, FocusHandle, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, ParentElement, Render, SharedString, Styled, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

actions!(
    tempo,
    [
        PlaySelected,
        TogglePause,
        MoveSelectionUp,
        MoveSelectionDown
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Index,
    Title,
    Album,
    Format,
    Plays,
    Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Ascending,
    Descending,
}

struct NavItem {
    label: &'static str,
    count: &'static str,
}

struct Track {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    year: &'static str,
    duration: &'static str,
    codec: &'static str,
    plays: &'static str,
    art: &'static str,
    art_color: u32,
    loved: bool,
}

const INDEX_COL_W: f32 = 34.0;
const ART_COL_W: f32 = 32.0;
const TITLE_COL_W: f32 = 188.0;
const ALBUM_COL_W: f32 = 230.0;
const FMT_COL_W: f32 = 70.0;
const PLAYS_COL_W: f32 = 82.0;
const TIME_COL_W: f32 = 64.0;
const LOVE_COL_W: f32 = 24.0;
const LEFT_SIDEBAR_W: f32 = 220.0;
const RIGHT_SIDEBAR_W: f32 = 300.0;

struct TempoApp {
    focus_handle: FocusHandle,
    page: Page,
    left_sidebar_collapsed: bool,
    right_sidebar_collapsed: bool,
    sort_column: SortColumn,
    sort_direction: SortDirection,
    selected_track: usize,
    playing_track: usize,
    is_playing: bool,
    context_menu_track: Option<usize>,
    context_menu_row: usize,
    tracks: Vec<Track>,
    queue: Vec<usize>,
}

impl TempoApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        Self {
            focus_handle,
            page: Page::Library,
            left_sidebar_collapsed: false,
            right_sidebar_collapsed: false,
            sort_column: SortColumn::Index,
            sort_direction: SortDirection::Ascending,
            selected_track: 9,
            playing_track: 9,
            is_playing: true,
            context_menu_track: None,
            context_menu_row: 0,
            tracks: vec![
                Track {
                    title: "Half-Light",
                    artist: "Mira Vale",
                    album: "Lantern Year",
                    year: "2025",
                    duration: "4:18",
                    codec: "FLAC",
                    plays: "87",
                    art: "LY",
                    art_color: 0x7b5735,
                    loved: false,
                },
                Track {
                    title: "Coast to Coast",
                    artist: "Mira Vale",
                    album: "Lantern Year",
                    year: "2025",
                    duration: "2:55",
                    codec: "FLAC",
                    plays: "53",
                    art: "LY",
                    art_color: 0x7b5735,
                    loved: false,
                },
                Track {
                    title: "Telegraph Hill",
                    artist: "Mira Vale",
                    album: "Lantern Year",
                    year: "2025",
                    duration: "5:11",
                    codec: "FLAC",
                    plays: "31",
                    art: "LY",
                    art_color: 0x7b5735,
                    loved: false,
                },
                Track {
                    title: "Stillwater",
                    artist: "North Pacific",
                    album: "North Pacific",
                    year: "2024",
                    duration: "6:24",
                    codec: "MP3",
                    plays: "219",
                    art: "NP",
                    art_color: 0x496777,
                    loved: false,
                },
                Track {
                    title: "Drift",
                    artist: "North Pacific",
                    album: "North Pacific",
                    year: "2024",
                    duration: "8:02",
                    codec: "MP3",
                    plays: "188",
                    art: "NP",
                    art_color: 0x496777,
                    loved: false,
                },
                Track {
                    title: "Cape Disappointment",
                    artist: "North Pacific",
                    album: "North Pacific",
                    year: "2024",
                    duration: "7:14",
                    codec: "MP3",
                    plays: "96",
                    art: "NP",
                    art_color: 0x496777,
                    loved: false,
                },
                Track {
                    title: "Crosswire",
                    artist: "Crosswire",
                    album: "Crosswire",
                    year: "2025",
                    duration: "3:28",
                    codec: "FLAC",
                    plays: "412",
                    art: "C",
                    art_color: 0x5b6b73,
                    loved: true,
                },
                Track {
                    title: "Foglight",
                    artist: "Crosswire",
                    album: "Crosswire",
                    year: "2025",
                    duration: "4:03",
                    codec: "FLAC",
                    plays: "274",
                    art: "C",
                    art_color: 0x5b6b73,
                    loved: false,
                },
                Track {
                    title: "Pier 31",
                    artist: "Crosswire",
                    album: "Crosswire",
                    year: "2025",
                    duration: "3:51",
                    codec: "FLAC",
                    plays: "198",
                    art: "C",
                    art_color: 0x5b6b73,
                    loved: false,
                },
                Track {
                    title: "Night Operator",
                    artist: "Nilo Park",
                    album: "Crosswire",
                    year: "2025",
                    duration: "5:44",
                    codec: "FLAC",
                    plays: "165",
                    art: "C",
                    art_color: 0x5b6b73,
                    loved: false,
                },
                Track {
                    title: "Slow Burn",
                    artist: "Nilo Park",
                    album: "Crosswire",
                    year: "2025",
                    duration: "4:22",
                    codec: "FLAC",
                    plays: "121",
                    art: "C",
                    art_color: 0x5b6b73,
                    loved: false,
                },
                Track {
                    title: "Morning, Eastbound",
                    artist: "Halverston",
                    album: "Field Recordings",
                    year: "2023",
                    duration: "4:47",
                    codec: "AAC",
                    plays: "64",
                    art: "FR",
                    art_color: 0x7d6c48,
                    loved: false,
                },
                Track {
                    title: "Forty-Seventh & Vine",
                    artist: "Halverston",
                    album: "Field Recordings",
                    year: "2023",
                    duration: "6:02",
                    codec: "AAC",
                    plays: "42",
                    art: "FR",
                    art_color: 0x7d6c48,
                    loved: false,
                },
                Track {
                    title: "A Letter To No One",
                    artist: "Halverston",
                    album: "Field Recordings",
                    year: "2023",
                    duration: "5:33",
                    codec: "AAC",
                    plays: "38",
                    art: "FR",
                    art_color: 0x7d6c48,
                    loved: false,
                },
                Track {
                    title: "Dovetail",
                    artist: "Otto Reyes",
                    album: "Dovetail",
                    year: "2022",
                    duration: "3:09",
                    codec: "FLAC",
                    plays: "311",
                    art: "D",
                    art_color: 0x8c5f55,
                    loved: true,
                },
                Track {
                    title: "Mission & 18th",
                    artist: "Otto Reyes",
                    album: "Dovetail",
                    year: "2022",
                    duration: "2:48",
                    codec: "FLAC",
                    plays: "254",
                    art: "D",
                    art_color: 0x8c5f55,
                    loved: false,
                },
                Track {
                    title: "House on Fillmore",
                    artist: "Otto Reyes",
                    album: "Dovetail",
                    year: "2022",
                    duration: "4:14",
                    codec: "FLAC",
                    plays: "207",
                    art: "D",
                    art_color: 0x8c5f55,
                    loved: false,
                },
                Track {
                    title: "Last Train Home",
                    artist: "Otto Reyes",
                    album: "Dovetail",
                    year: "2022",
                    duration: "5:52",
                    codec: "FLAC",
                    plays: "178",
                    art: "D",
                    art_color: 0x8c5f55,
                    loved: false,
                },
                Track {
                    title: "Nocturne in B",
                    artist: "Iva Tassen",
                    album: "Nocturnes",
                    year: "2021",
                    duration: "7:22",
                    codec: "FLAC",
                    plays: "92",
                    art: "N",
                    art_color: 0x55536f,
                    loved: false,
                },
                Track {
                    title: "Etude No. 4",
                    artist: "Iva Tassen",
                    album: "Nocturnes",
                    year: "2021",
                    duration: "4:36",
                    codec: "FLAC",
                    plays: "71",
                    art: "N",
                    art_color: 0x55536f,
                    loved: false,
                },
                Track {
                    title: "Largo",
                    artist: "Iva Tassen",
                    album: "Nocturnes",
                    year: "2021",
                    duration: "8:51",
                    codec: "FLAC",
                    plays: "46",
                    art: "N",
                    art_color: 0x55536f,
                    loved: false,
                },
            ],
            queue: vec![9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 0, 1, 2],
        }
    }
}

impl Render for TempoApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tempo-app")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::play_selected))
            .on_action(cx.listener(Self::toggle_pause))
            .on_action(cx.listener(Self::move_selection_up))
            .on_action(cx.listener(Self::move_selection_down))
            .size_full()
            .bg(rgb(0x111216))
            .text_color(rgb(0xd8d8dd))
            .font_family("Inter")
            .text_sm()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_left_sidebar(cx))
                    .child(self.render_content(cx)),
            )
            .child(self.render_player_bar())
    }
}

impl TempoApp {
    fn play_selected(&mut self, _: &PlaySelected, _: &mut Window, cx: &mut Context<Self>) {
        self.playing_track = self.selected_track;
        self.is_playing = true;
        self.context_menu_track = None;
        cx.notify();
    }

    fn toggle_pause(&mut self, _: &TogglePause, _: &mut Window, cx: &mut Context<Self>) {
        self.is_playing = !self.is_playing;
        cx.notify();
    }

    fn move_selection_up(&mut self, _: &MoveSelectionUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1);
        cx.notify();
    }

    fn move_selection_down(
        &mut self,
        _: &MoveSelectionDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(1);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize) {
        let indices = self.sorted_track_indices();
        let Some(position) = indices.iter().position(|ix| *ix == self.selected_track) else {
            return;
        };
        let next = (position as isize + delta).clamp(0, indices.len().saturating_sub(1) as isize);
        self.selected_track = indices[next as usize];
        self.context_menu_track = None;
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::Library => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_library(cx))
                .child(self.render_queue(cx))
                .into_any_element(),
            Page::Settings => self.render_settings(cx).into_any_element(),
        }
    }

    fn render_left_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let collapsed = self.left_sidebar_collapsed;

        if collapsed {
            return div().w(px(0.0)).flex_none().into_any_element();
        }

        div()
            .w(px(LEFT_SIDEBAR_W))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_r_1()
            .border_color(rgb(0x24252b))
            .bg(rgb(0x15161a))
            .child(
                div()
                    .w(px(LEFT_SIDEBAR_W))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.render_sidebar_header(cx))
                    .child(self.render_nav_group(
                        "LIBRARY",
                        [
                            NavItem {
                                label: "All Music",
                                count: "22",
                            },
                            NavItem {
                                label: "Recently Added",
                                count: "8",
                            },
                            NavItem {
                                label: "Most Played",
                                count: "12",
                            },
                            NavItem {
                                label: "Unrated",
                                count: "3",
                            },
                        ],
                        cx,
                    ))
                    .child(self.render_nav_group(
                        "PLAYLISTS",
                        [
                            NavItem {
                                label: "Deep Focus",
                                count: "14",
                            },
                            NavItem {
                                label: "After Hours",
                                count: "21",
                            },
                            NavItem {
                                label: "Long Walk",
                                count: "18",
                            },
                            NavItem {
                                label: "Q4 Mix",
                                count: "9",
                            },
                        ],
                        cx,
                    ))
                    .child(self.render_nav_group(
                        "DEVICES",
                        [
                            NavItem {
                                label: "This Mac",
                                count: "1,842",
                            },
                            NavItem {
                                label: "External SSD",
                                count: "7,361",
                            },
                        ],
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(rgb(0x24252b))
                            .text_xs()
                            .text_color(rgb(0x6f737c))
                            .flex()
                            .justify_between()
                            .child("9,225 tracks")
                            .child("43.2 GB"),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(50.0))
            .flex()
            .items_center()
            .px_4()
            .border_b_1()
            .border_color(rgb(0x1e2026))
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf0f0f4))
                    .child("Tempo"),
            )
            .child(
                Self::sidebar_button("‹", "toggle-left-sidebar").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.left_sidebar_collapsed = !this.left_sidebar_collapsed;
                        cx.notify();
                    },
                )),
            )
    }

    fn sidebar_button(label: &'static str, id: &'static str) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .w(px(24.0))
            .h(px(24.0))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x30323a))
            .bg(rgb(0x1b1c22))
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(0x9a9ea8))
            .active(|this| this.opacity(0.82))
            .child(label)
    }

    fn render_nav_group<const N: usize>(
        &self,
        title: &'static str,
        items: [NavItem; N],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .px_3()
            .pb_4()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x666a73))
                    .child(title),
            )
            .children(items.into_iter().map(|item| {
                let active = self.page == Page::Library && item.label == "All Music";
                self.render_nav_item(item.label, item.count, active, Page::Library, cx)
            }))
            .when(title == "LIBRARY", |this| {
                this.child(self.render_nav_item(
                    "Settings",
                    "",
                    self.page == Page::Settings,
                    Page::Settings,
                    cx,
                ))
            })
    }

    fn render_nav_item(
        &self,
        label: &'static str,
        count: &'static str,
        active: bool,
        target: Page,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bg = if active { 0x282a30 } else { 0x15161a };
        let fg = if active { 0xf0f0f4 } else { 0xb6b8bf };

        div()
            .id(SharedString::from(format!("nav-{label}")))
            .h(px(22.0))
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .active(|this| this.opacity(0.82))
            .child(label)
            .child(div().text_xs().text_color(rgb(0x777b84)).child(count))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.page = target;
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_library(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(0x131419))
            .child(self.render_library_header(cx))
            .child(self.render_table(cx))
    }

    fn render_library_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(54.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .border_b_1()
            .border_color(rgb(0x24252b))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.left_sidebar_collapsed, |this| {
                        this.child(Self::sidebar_button("›", "open-left-sidebar").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.left_sidebar_collapsed = false;
                                cx.notify();
                            }),
                        ))
                    })
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0xf0f0f4))
                            .child("All Music"),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x676b74))
                    .child("22 items  ·  1h 47m"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(px(180.0))
                    .h(px(26.0))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0x30323a))
                    .bg(rgb(0x18191f))
                    .px_3()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(0x737781))
                    .child("⌕  Search library"),
            )
            .when(self.right_sidebar_collapsed, |this| {
                this.child(
                    Self::sidebar_button("‹", "open-right-sidebar").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.right_sidebar_collapsed = false;
                            cx.notify();
                        },
                    )),
                )
            })
    }

    fn render_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let indices = self.sorted_track_indices();

        div()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .child(self.render_table_header(cx))
            .children(indices.into_iter().enumerate().map(|(row_ix, track_ix)| {
                self.render_track_row(row_ix, track_ix, &self.tracks[track_ix], cx)
            }))
            .when_some(self.context_menu_track, |this, track_ix| {
                this.child(self.render_context_menu(track_ix))
            })
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .h(px(27.0))
            .px_4()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(0x24252b))
            .text_xs()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(rgb(0x5f636c))
            .child(self.header_cell("#", INDEX_COL_W, SortColumn::Index, cx))
            .child(Self::static_header_cell("", ART_COL_W))
            .child(self.header_cell("TITLE", TITLE_COL_W, SortColumn::Title, cx))
            .child(self.header_cell("ALBUM", ALBUM_COL_W, SortColumn::Album, cx))
            .child(self.header_cell("FMT", FMT_COL_W, SortColumn::Format, cx))
            .child(self.header_cell("PLAYS", PLAYS_COL_W, SortColumn::Plays, cx))
            .child(self.header_cell("TIME", TIME_COL_W, SortColumn::Duration, cx))
            .child(Self::static_header_cell("", LOVE_COL_W))
    }

    fn header_cell(
        &self,
        label: &'static str,
        width: f32,
        column: SortColumn,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.sort_column == column;
        let icon = match self.sort_direction {
            SortDirection::Ascending => "▲",
            SortDirection::Descending => "▼",
        };
        let id = match column {
            SortColumn::Index => "sort-index",
            SortColumn::Title => "sort-title",
            SortColumn::Album => "sort-album",
            SortColumn::Format => "sort-format",
            SortColumn::Plays => "sort-plays",
            SortColumn::Duration => "sort-duration",
        };

        div()
            .id(id)
            .w(px(width))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_1()
            .text_color(rgb(if active { 0xc9ccd4 } else { 0x5f636c }))
            .hover(|this| this.text_color(rgb(0xc9ccd4)))
            .child(label)
            .when(active, |this| this.child(icon))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.sort_column == column {
                    this.sort_direction = match this.sort_direction {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    };
                } else {
                    this.sort_column = column;
                    this.sort_direction = SortDirection::Ascending;
                }

                cx.notify();
            }))
    }

    fn static_header_cell(label: &'static str, width: f32) -> impl IntoElement {
        div().w(px(width)).child(label)
    }

    fn sorted_track_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.tracks.len()).collect();

        indices.sort_by(|a, b| {
            let left = &self.tracks[*a];
            let right = &self.tracks[*b];
            let ordering = match self.sort_column {
                SortColumn::Index => a.cmp(b),
                SortColumn::Title => left.title.cmp(right.title),
                SortColumn::Album => left
                    .album
                    .cmp(right.album)
                    .then(left.title.cmp(right.title)),
                SortColumn::Format => left
                    .codec
                    .cmp(right.codec)
                    .then(left.title.cmp(right.title)),
                SortColumn::Plays => left
                    .plays
                    .parse::<u32>()
                    .unwrap_or_default()
                    .cmp(&right.plays.parse::<u32>().unwrap_or_default()),
                SortColumn::Duration => Self::duration_seconds(left.duration)
                    .cmp(&Self::duration_seconds(right.duration)),
            };

            match self.sort_direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        });

        indices
    }

    fn duration_seconds(duration: &str) -> u32 {
        let Some((minutes, seconds)) = duration.split_once(':') else {
            return 0;
        };

        minutes.parse::<u32>().unwrap_or_default() * 60 + seconds.parse::<u32>().unwrap_or_default()
    }

    fn render_track_row(
        &self,
        row_ix: usize,
        track_ix: usize,
        track: &Track,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = track_ix == self.playing_track;
        let selected = track_ix == self.selected_track;
        let bg = if selected {
            0x30323a
        } else if active {
            0x25262c
        } else {
            0x131419
        };
        let title_color = if active { 0xeeb17d } else { 0xe2e2e7 };

        div()
            .id(SharedString::from(format!("track-row-{track_ix}")))
            .h(px(32.0))
            .px_4()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(0x202127))
            .bg(rgb(bg))
            .cursor_pointer()
            .hover(|this| this.bg(rgb(0x202229)))
            .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                this.selected_track = track_ix;
                this.context_menu_track = None;

                if event.standard_click() && event.click_count() >= 2 {
                    this.playing_track = track_ix;
                    this.is_playing = true;
                }

                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                    this.selected_track = track_ix;
                    this.context_menu_track = Some(track_ix);
                    this.context_menu_row = row_ix;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(INDEX_COL_W))
                    .text_xs()
                    .text_color(rgb(0x6d717a))
                    .child(if active {
                        if self.is_playing { "Ⅱ" } else { "▶" }.into()
                    } else {
                        format!("{:02}", track_ix + 1)
                    }),
            )
            .child(
                div()
                    .w(px(ART_COL_W))
                    .flex()
                    .items_center()
                    .child(Self::album_tile(track, 22.0)),
            )
            .child(
                div()
                    .w(px(TITLE_COL_W))
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(title_color))
                    .child(track.title),
            )
            .child(Self::cell(track.album, ALBUM_COL_W))
            .child(Self::cell(track.codec, FMT_COL_W))
            .child(Self::cell(track.plays, PLAYS_COL_W))
            .child(Self::cell(track.duration, TIME_COL_W))
            .child(
                div()
                    .w(px(LOVE_COL_W))
                    .text_color(rgb(0xf0b282))
                    .child(if track.loved { "♥" } else { "" }),
            )
    }

    fn cell(content: impl Into<SharedString>, width: f32) -> impl IntoElement {
        div()
            .w(px(width))
            .overflow_hidden()
            .text_ellipsis()
            .text_color(rgb(0x8a8e97))
            .child(content.into())
    }

    fn render_context_menu(&self, track_ix: usize) -> impl IntoElement {
        let track = &self.tracks[track_ix];
        let top = 27.0 + ((self.context_menu_row as f32 + 1.0) * 32.0).min(560.0);

        div()
            .absolute()
            .top(px(top))
            .left(px(76.0))
            .w(px(190.0))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x343741))
            .bg(rgb(0x1b1c22))
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x2b2d35))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf0f0f4))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(track.title),
            )
            .child(Self::context_menu_item("Play from start"))
            .child(Self::context_menu_item("Add to queue"))
            .child(Self::context_menu_item("Go to album"))
            .child(Self::context_menu_item("Show file"))
    }

    fn context_menu_item(label: &'static str) -> impl IntoElement {
        div()
            .h(px(28.0))
            .px_3()
            .flex()
            .items_center()
            .cursor_pointer()
            .text_color(rgb(0xc9ccd4))
            .hover(|this| this.bg(rgb(0x282a30)).text_color(rgb(0xf0f0f4)))
            .child(label)
    }

    fn album_tile(track: &Track, size: f32) -> impl IntoElement {
        div()
            .w(px(size))
            .h(px(size))
            .rounded_sm()
            .bg(rgb(track.art_color))
            .border_1()
            .border_color(rgb(0x3a3d45))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(0xf4f0ea))
            .child(track.art)
    }

    fn render_queue(&self, cx: &mut Context<Self>) -> AnyElement {
        let collapsed = self.right_sidebar_collapsed;

        if collapsed {
            return div().w(px(0.0)).flex_none().into_any_element();
        }

        div()
            .w(px(RIGHT_SIDEBAR_W))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_l_1()
            .border_color(rgb(0x24252b))
            .bg(rgb(0x17161b))
            .child(
                div()
                    .w(px(RIGHT_SIDEBAR_W))
                    .h(px(54.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(0x24252b))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Self::sidebar_button("›", "toggle-right-sidebar").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.right_sidebar_collapsed = !this.right_sidebar_collapsed;
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xf0f0f4))
                                    .child("Up Next"),
                            ),
                    )
                    .child(div().text_xs().text_color(rgb(0x676b74)).child("16 tracks")),
            )
            .child(
                div().w(px(RIGHT_SIDEBAR_W)).children(
                    self.queue
                        .iter()
                        .enumerate()
                        .map(|(ix, track_ix)| self.render_queue_row(ix, &self.tracks[*track_ix])),
                ),
            )
            .into_any_element()
    }

    fn render_queue_row(&self, ix: usize, track: &Track) -> impl IntoElement {
        let active = ix == 0;
        let bg = if active { 0x242329 } else { 0x17161b };

        div()
            .h(px(41.0))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .bg(rgb(bg))
            .child(
                div()
                    .w(px(22.0))
                    .text_xs()
                    .text_color(rgb(0x70747d))
                    .child(format!("{}", ix + 1)),
            )
            .child(Self::album_tile(track, 28.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(if active { 0xeeb17d } else { 0xe2e2e7 }))
                            .child(track.title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x878b94))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(track.artist),
                    ),
            )
            .child(
                div()
                    .w(px(42.0))
                    .text_xs()
                    .text_color(rgb(0x777b84))
                    .child(track.duration),
            )
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .bg(rgb(0x131419))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(54.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0x24252b))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(self.left_sidebar_collapsed, |this| {
                                this.child(Self::sidebar_button("›", "open-left-sidebar").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.left_sidebar_collapsed = false;
                                        cx.notify();
                                    }),
                                ))
                            })
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xf0f0f4))
                                    .child("Settings"),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-back")
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x30323a))
                            .bg(rgb(0x1b1c22))
                            .active(|this| this.opacity(0.82))
                            .child("Back to Library")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = Page::Library;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(Self::settings_section(
                        "Library",
                        [
                            ("Music folders", "Add folders and monitor changes"),
                            ("Scanner", "Background indexing with hidden files ignored"),
                        ],
                    ))
                    .child(Self::settings_section(
                        "Playback",
                        [
                            ("Output", "Default CPAL output device"),
                            ("ReplayGain", "Deferred: off, track, album, smart"),
                            ("Random", "Available from player bar and tray menu"),
                        ],
                    ))
                    .child(Self::settings_section(
                        "Interface",
                        [
                            ("Density", "Compact, table-first layout"),
                            ("UI scale", "GPUI rem scaling setting to wire next"),
                        ],
                    ))
                    .child(Self::settings_section(
                        "Desktop Integration",
                        [
                            ("MPRIS", "Planned for media keys and shell widgets"),
                            ("Taskbar icon", "Use ksni StatusNotifierItem on Linux"),
                        ],
                    )),
            )
    }

    fn settings_section<const N: usize>(
        title: &'static str,
        rows: [(&'static str, &'static str); N],
    ) -> impl IntoElement {
        div()
            .rounded_lg()
            .border_1()
            .border_color(rgb(0x24252b))
            .bg(rgb(0x17181e))
            .overflow_hidden()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .bg(rgb(0x1b1c22))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(title),
            )
            .children(rows.into_iter().map(|(label, value)| {
                div()
                    .h(px(36.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgb(0x24252b))
                    .child(label)
                    .child(div().text_xs().text_color(rgb(0x858993)).child(value))
            }))
    }

    fn render_player_bar(&self) -> impl IntoElement {
        let track = &self.tracks[self.playing_track];

        div()
            .h(px(86.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .border_t_1()
            .border_color(rgb(0x282a30))
            .bg(rgb(0x18191e))
            .child(Self::album_tile(track, 54.0))
            .child(
                div()
                    .w(px(220.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0xf0f0f4))
                            .child(track.title),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x9a9ea8))
                            .child(format!("{} - {}", track.artist, track.album)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x70747d))
                            .child(format!("{}  ·  1411 kbps  ·  {}", track.codec, track.year)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .relative()
                    .child(Self::waveform_seekbar(track.duration)),
            )
            .child(
                div()
                    .w(px(170.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_color(rgb(0xa6aab4))
                    .child(Self::transport_overlay(self.is_playing))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child("☰")
                            .child("♩")
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(3.0))
                                    .rounded_full()
                                    .bg(rgb(0x777b84))
                                    .child(
                                        div()
                                            .w(px(104.0))
                                            .h(px(3.0))
                                            .rounded_full()
                                            .bg(rgb(0xd8d8dd)),
                                    ),
                            ),
                    ),
            )
    }

    fn waveform_seekbar(duration: &'static str) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .rounded_lg()
            .overflow_hidden()
            .bg(rgb(0x15161b))
            .border_1()
            .border_color(rgb(0x24252b))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(210.0))
                    .bg(rgb(0x1d2230)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children((0..120).map(Self::waveform_bar)),
            )
            .child(
                div()
                    .absolute()
                    .bottom_2()
                    .left_3()
                    .text_xs()
                    .text_color(rgb(0x777b84))
                    .child("0:01"),
            )
            .child(
                div()
                    .absolute()
                    .bottom_2()
                    .right_3()
                    .text_xs()
                    .text_color(rgb(0x777b84))
                    .child(duration),
            )
    }

    fn waveform_bar(ix: usize) -> impl IntoElement {
        let heights = [
            10.0, 18.0, 27.0, 16.0, 34.0, 22.0, 42.0, 29.0, 14.0, 36.0, 20.0, 31.0,
        ];
        let height = heights[ix % heights.len()];
        let color = if ix < 36 { 0x6f8fd9 } else { 0x333640 };

        div()
            .flex_1()
            .min_w(px(2.0))
            .max_w(px(3.0))
            .h(px(height))
            .rounded_full()
            .bg(rgb(color))
    }

    fn transport_overlay(is_playing: bool) -> impl IntoElement {
        div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_full()
            .bg(rgb(0x111216))
            .border_1()
            .border_color(rgb(0x30323a))
            .child(Self::transport_button("⌘", false))
            .child(Self::transport_button("◀", false))
            .child(Self::transport_button(
                if is_playing { "Ⅱ" } else { "▶" },
                true,
            ))
            .child(Self::transport_button("▶", false))
            .child(Self::transport_button("↻", false))
    }

    fn transport_button(label: &'static str, primary: bool) -> impl IntoElement {
        let size = if primary { 28.0 } else { 22.0 };
        let hover_size = if primary { 32.0 } else { 26.0 };
        let bg = if primary { 0xe7e7ea } else { 0x18191e };
        let fg = if primary { 0x111216 } else { 0x9a9ea8 };

        div()
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
                    .bg(rgb(0xf0f0f4))
                    .text_color(rgb(0x111216))
            })
            .child(label)
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("tempo".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TempoApp::new(window, cx)),
        )
        .expect("failed to open Tempo window");

        cx.bind_keys([
            KeyBinding::new("enter", PlaySelected, None),
            KeyBinding::new("space", TogglePause, None),
            KeyBinding::new("left", MoveSelectionUp, None),
            KeyBinding::new("right", MoveSelectionDown, None),
        ]);

        cx.activate(true);
    });
}
