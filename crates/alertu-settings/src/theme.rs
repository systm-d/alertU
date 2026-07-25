//! The website's palette, ported to egui.
//!
//! Values are copied from the `:root` block of `site/sass/main.scss` so the
//! window and the site read as one product. Keep them in step: this is a copy,
//! not a shared source, because a settings window has no business parsing SCSS
//! at runtime.
//!
//! The site already defines three "car-alarm state" colours, and they map one to
//! one onto `GuardState` — which is why the state badge here needs no palette of
//! its own.

use alertu_common::state::GuardState;
use eframe::egui::{self, Color32, Rounding, Stroke, Visuals};

// Brand.
pub const BLUE: Color32 = Color32::from_rgb(0x2b, 0x6b, 0xff);
pub const BLUE_INK: Color32 = Color32::from_rgb(0x1b, 0x4f, 0xd6);
pub const BLUE_50: Color32 = Color32::from_rgb(0xee, 0xf4, 0xff);
pub const BLUE_100: Color32 = Color32::from_rgb(0xdb, 0xe7, 0xff);

// Neutrals.
pub const INK: Color32 = Color32::from_rgb(0x0c, 0x15, 0x26);
pub const INK_2: Color32 = Color32::from_rgb(0x37, 0x46, 0x5c);
pub const MUT: Color32 = Color32::from_rgb(0x6b, 0x7a, 0x90);
pub const BG: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const BG_SOFT: Color32 = Color32::from_rgb(0xf5, 0xf8, 0xfc);
pub const LINE: Color32 = Color32::from_rgb(0xe8, 0xee, 0xf6);
pub const LINE_2: Color32 = Color32::from_rgb(0xd7, 0xe0, 0xee);

// Car-alarm states.
pub const SAFE: Color32 = Color32::from_rgb(0x1f, 0xb8, 0x77);
pub const ARMED: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x2e);
pub const ALARM: Color32 = Color32::from_rgb(0xf2, 0x4a, 0x63);

/// Corner radius shared by widgets, matching the site's rounded cards.
const RADIUS: f32 = 6.0;

/// The colour the site would use for `state`.
///
/// `Triggered` deliberately reuses `ARMED` rather than `ALARM`: the countdown is
/// still cancellable, and colouring it like a firing siren would misreport how
/// bad things are.
pub fn state_color(state: GuardState) -> Color32 {
    match state {
        GuardState::Idle => SAFE,
        GuardState::Armed | GuardState::Triggered => ARMED,
        GuardState::Alarm => ALARM,
    }
}

/// A short label for `state`, for the header badge.
pub fn state_label(state: GuardState) -> &'static str {
    match state {
        GuardState::Idle => "Disarmed",
        GuardState::Armed => "Armed",
        GuardState::Triggered => "Intrusion — counting down",
        GuardState::Alarm => "ALARM",
    }
}

/// Install the palette on `ctx`.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::light();

    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.faint_bg_color = BG_SOFT;
    visuals.extreme_bg_color = BG_SOFT;
    visuals.hyperlink_color = BLUE;
    visuals.window_stroke = Stroke::new(1.0_f32, LINE);
    visuals.window_rounding = Rounding::same(10.0);
    visuals.selection.bg_fill = BLUE_100;
    visuals.selection.stroke = Stroke::new(1.0_f32, BLUE_INK);

    // Resting widgets: soft background, hairline border, ink text — the site's
    // input and button styling.
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = BG_SOFT;
    w.noninteractive.weak_bg_fill = BG_SOFT;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUT);
    w.noninteractive.rounding = Rounding::same(RADIUS);

    w.inactive.bg_fill = BG_SOFT;
    w.inactive.weak_bg_fill = BG_SOFT;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, LINE_2);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, INK_2);
    w.inactive.rounding = Rounding::same(RADIUS);

    w.hovered.bg_fill = BLUE_50;
    w.hovered.weak_bg_fill = BLUE_50;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, BLUE);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, INK);
    w.hovered.rounding = Rounding::same(RADIUS);

    w.active.bg_fill = BLUE_100;
    w.active.weak_bg_fill = BLUE_100;
    w.active.bg_stroke = Stroke::new(1.0_f32, BLUE_INK);
    w.active.fg_stroke = Stroke::new(1.0_f32, INK);
    w.active.rounding = Rounding::same(RADIUS);

    w.open.bg_fill = BG_SOFT;
    w.open.weak_bg_fill = BG_SOFT;
    w.open.bg_stroke = Stroke::new(1.0_f32, LINE_2);
    w.open.fg_stroke = Stroke::new(1.0_f32, INK);
    w.open.rounding = Rounding::same(RADIUS);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_colour_and_a_label() {
        for state in [
            GuardState::Idle,
            GuardState::Armed,
            GuardState::Triggered,
            GuardState::Alarm,
        ] {
            assert!(!state_label(state).is_empty());
        }
        assert_eq!(state_color(GuardState::Idle), SAFE);
        assert_eq!(state_color(GuardState::Alarm), ALARM);
        // Triggered is still cancellable, so it must not look like a firing siren.
        assert_eq!(state_color(GuardState::Triggered), ARMED);
    }
}
