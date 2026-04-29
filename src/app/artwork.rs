//! Album artwork tile rendering helpers.
//!
//! These are free functions (rather than methods on [`TempoApp`])
//! because the player bar — which lives in its own [`PlayerEntity`]
//! after the Phase 3 #16 split — also renders an album tile in the
//! Now-Playing strip. Both entities call into these helpers without
//! borrowing each other.

use super::*;

pub(super) fn album_tile(track: &Track, size: f32, colors: ThemeColors) -> AnyElement {
    album_tile_with_hover_border(track, size, None, colors)
}

pub(super) fn album_tile_with_hover_border(
    track: &Track,
    size: f32,
    hover_border: Option<u32>,
    colors: ThemeColors,
) -> AnyElement {
    let initials = track.album_initials.clone();
    let color = track.album_color;
    let fallback_initials = initials.clone();
    let mut tile = div()
        .w(px(size))
        .h(px(size))
        .rounded_sm()
        .border_1()
        .border_color(rgb(colors.border_strong))
        .overflow_hidden();

    if let Some(hover_border) = hover_border {
        tile = tile.hover(move |this| this.border_color(rgb(hover_border)));
    }

    tile.child(match &track.artwork {
        Some(TrackArtwork::Embedded(image)) => img(image.clone())
            .size_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(move || album_tile_fallback(fallback_initials.clone(), color, colors))
            .into_any_element(),
        Some(TrackArtwork::File(path)) => img(path.clone())
            .size_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback(move || album_tile_fallback(fallback_initials.clone(), color, colors))
            .into_any_element(),
        None => album_tile_fallback(initials, color, colors),
    })
    .into_any_element()
}

pub(super) fn album_tile_placeholder(track: &Track, size: f32, colors: ThemeColors) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .rounded_sm()
        .border_1()
        .border_color(rgb(colors.border_strong))
        .overflow_hidden()
        .child(album_tile_fallback(
            track.album_initials.clone(),
            track.album_color,
            colors,
        ))
        .into_any_element()
}

pub(super) fn album_tile_fallback(initials: String, color: u32, colors: ThemeColors) -> AnyElement {
    div()
        .size_full()
        .bg(rgb(color))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(rgb(colors.album_tile_text))
        .child(initials)
        .into_any_element()
}

pub(super) fn album_initials_for(album: &str, title: &str) -> String {
    let source = if album == "Unknown Album" {
        title
    } else {
        album
    };

    initials_for(source)
}

pub(super) fn album_color_for(album: &str, artist: &str) -> u32 {
    color_for(album, artist)
}

pub(super) fn initials_for(value: &str) -> String {
    let mut initials = value
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    if initials.is_empty() {
        initials.push('?');
    }

    initials
}

pub(super) fn color_for(value: &str, salt: &str) -> u32 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.bytes().chain(salt.bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    let palette = [
        0x7b5735, 0x496777, 0x5b6b73, 0x7d6c48, 0x8c5f55, 0x55536f, 0x42685f, 0x744f6d,
    ];
    palette[(hash as usize) % palette.len()]
}
