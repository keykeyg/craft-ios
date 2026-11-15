//! craft-ios touch layer: iOS-sized controls for every egui app, applied by eframe each frame.
//!
//! Floors, not overrides: anything an app already draws bigger stays as it is, and an app's own
//! colours and theme switches flow through. Numbers follow Apple's Human Interface Guidelines:
//! 44x44 pt minimum hit targets, 8 pt between targets, 17 pt body text.

use egui::{CornerRadius, Style, TextStyle};

const TAP: f32 = 44.0;
const GAP: f32 = 8.0;
const CAPSULE: u8 = 22;

fn at_least(v: &mut f32, min: f32) {
    if *v < min {
        *v = min;
    }
}

fn touch_style(s: &mut Style) {
    let sp = &mut s.spacing;
    at_least(&mut sp.interact_size.x, TAP);
    at_least(&mut sp.interact_size.y, TAP);
    at_least(&mut sp.button_padding.x, 14.0);
    at_least(&mut sp.button_padding.y, 10.0);
    at_least(&mut sp.item_spacing.x, GAP);
    at_least(&mut sp.item_spacing.y, GAP);
    at_least(&mut sp.icon_width, 24.0);
    at_least(&mut sp.icon_width_inner, 14.0);
    at_least(&mut sp.icon_spacing, GAP);
    at_least(&mut sp.scroll.bar_width, 8.0);

    for (style, font) in s.text_styles.iter_mut() {
        let min = match style {
            TextStyle::Small => 13.0,
            TextStyle::Body | TextStyle::Button => 17.0,
            TextStyle::Monospace => 15.0,
            TextStyle::Heading => 22.0,
            TextStyle::Name(_) => 15.0,
        };
        at_least(&mut font.size, min);
    }

    // Rounded "bubble" controls; panels and frames keep the app's own shape.
    let w = &mut s.visuals.widgets;
    for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = CornerRadius::same(CAPSULE);
    }
    s.visuals.menu_corner_radius = CornerRadius::same(14);
    s.visuals.window_corner_radius = CornerRadius::same(16);

    at_least(&mut s.interaction.resize_grab_radius_side, 12.0);
    at_least(&mut s.interaction.resize_grab_radius_corner, 16.0);
}

/// Bring the context's styles up to touch size. Writes only when something changed, so an
/// unchanged style isn't replaced (and caches keyed on it stay warm).
pub(crate) fn apply(ctx: &egui::Context) {
    let needs_update = [egui::Theme::Dark, egui::Theme::Light].into_iter().any(|theme| {
        let current = ctx.style_of(theme);
        let mut wanted = (*current).clone();
        touch_style(&mut wanted);
        wanted != *current
    });
    if needs_update {
        ctx.all_styles_mut(touch_style);
    }
}
