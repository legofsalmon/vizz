//! egui's own widgets, drawn in the shared states.
//!
//! Most of vizz is painted by hand from the tokens, but buttons, ticks,
//! fields, sliders, menus and windows are egui's, and egui draws them
//! from its `Visuals`. Left at `Visuals::dark()` they were a second
//! design language in the same window: a blue selection that was not the
//! current-item colour, grey buttons with no edge, a focus ring nobody
//! chose. [`apply`] sets them from the shared roles so the two agree.
//!
//! The states map onto egui's four as the shared state model draws them:
//! rest is raised, hover goes to raised-hover, pressed sinks to the well,
//! a selection is the accent's wash with an accent edge, and disabled is
//! the faint ink. egui draws keyboard focus with the pressed style, so the
//! pressed style's edge is the focus ring. It also draws a button's edge
//! and a checkbox's box with one stroke, and a box has to be found at
//! 3:1, so both take the control edge.

use egui::{CornerRadius, Shadow, Stroke};

use crate::tokens::{color::dark as role, elevation, radius::compact as radius, space};
use crate::{ink, motion, rgba, surface};

/// Sets vizz's visuals and the motion and hover timings on `ctx`.
pub fn apply(ctx: &egui::Context) {
    // Dark whatever the OS is set to: the room is dark either way.
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.set_visuals_of(egui::Theme::Light, visuals());
    let mode = motion::mode();
    ctx.all_styles_mut(|style| {
        // A fold opening is a small thing arriving near where it was asked
        // for: the base duration. Under reduced motion it is shorter, and
        // egui's folds only grow in place, never travel.
        style.animation_time = mode.base;
        style.interaction.tooltip_delay = motion::TOOLTIP;
    });
}

/// The visuals on their own, for a preview that draws without `apply`.
pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let control = CornerRadius::same(radius::CONTROL as u8);
    let overlay = CornerRadius::same(radius::OVERLAY as u8);
    let hairline = space::HAIRLINE;

    // Plain text is what the screen is about; weak text is the supporting
    // line. Warnings and errors written by egui itself take the inks.
    v.override_text_color = None;
    v.weak_text_color = Some(ink::SECONDARY);
    v.hyperlink_color = rgba(role::accent::INK);
    v.warn_fg_color = rgba(role::warn::INK);
    v.error_fg_color = rgba(role::bad::INK);

    v.panel_fill = surface::BASE;
    v.faint_bg_color = surface::WELL;
    v.extreme_bg_color = surface::GROOVE;
    v.text_edit_bg_color = Some(surface::WELL);
    v.code_bg_color = surface::WELL;
    v.window_fill = surface::OVERLAY;
    v.window_stroke = Stroke::new(hairline, rgba(role::line::STRONG));
    v.window_corner_radius = overlay;
    v.menu_corner_radius = overlay;
    v.window_shadow = shadow(elevation::dark::MODAL);
    v.popup_shadow = shadow(elevation::dark::MENU);

    // The selection — a chosen format, a toggle that is on, selected text:
    // the accent's wash, and its ink on top.
    v.selection.bg_fill = rgba(role::accent::SOFT);
    v.selection.stroke = Stroke::new(hairline, rgba(role::accent::INK));

    let w = &mut v.widgets;
    // Labels, separators, frames: structure, not controls.
    w.noninteractive.bg_fill = surface::BASE;
    w.noninteractive.weak_bg_fill = surface::BASE;
    w.noninteractive.bg_stroke = Stroke::new(hairline, surface::HAIRLINE);
    w.noninteractive.fg_stroke = Stroke::new(hairline, ink::PRIMARY);
    w.noninteractive.corner_radius = control;
    // At rest.
    w.inactive.bg_fill = surface::RAISED;
    w.inactive.weak_bg_fill = surface::RAISED;
    w.inactive.bg_stroke = Stroke::new(hairline, surface::CONTROL_EDGE);
    w.inactive.fg_stroke = Stroke::new(hairline, ink::PRIMARY);
    w.inactive.corner_radius = control;
    w.inactive.expansion = 0.0;
    // Under the pointer: the surface lifts and the edge firms, nothing
    // grows, so nothing moves under the hand.
    w.hovered.bg_fill = surface::RAISED_HOVER;
    w.hovered.weak_bg_fill = surface::RAISED_HOVER;
    w.hovered.bg_stroke = Stroke::new(hairline, surface::HOVER_EDGE);
    w.hovered.fg_stroke = Stroke::new(hairline, ink::PRIMARY);
    w.hovered.corner_radius = control;
    w.hovered.expansion = 0.0;
    // Held down, and keyboard focus: the control sinks into the well, and
    // the focus ring is its edge.
    w.active.bg_fill = surface::WELL;
    w.active.weak_bg_fill = surface::WELL;
    w.active.bg_stroke = Stroke::new(space::HALF, surface::FOCUS);
    w.active.fg_stroke = Stroke::new(hairline, ink::PRIMARY);
    w.active.corner_radius = control;
    w.active.expansion = 0.0;
    // A menu or a combo box that is open.
    w.open.bg_fill = surface::RAISED_HOVER;
    w.open.weak_bg_fill = surface::RAISED_HOVER;
    w.open.bg_stroke = Stroke::new(hairline, surface::HOVER_EDGE);
    w.open.fg_stroke = Stroke::new(hairline, ink::PRIMARY);
    w.open.corner_radius = control;

    v
}

/// A shadow from the system, as egui takes it: whole points both ways.
fn shadow(s: crate::tokens::Shadow) -> Shadow {
    Shadow {
        offset: [s.offset_x as i8, s.offset_y as i8],
        blur: s.blur as u8,
        spread: s.spread as u8,
        color: rgba(s.color),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selection is the current-item colour's family, not egui's own
    /// blue: one meaning, one colour, across hand-painted and egui widgets.
    #[test]
    fn the_selection_is_the_accent() {
        let v = visuals();
        assert_eq!(v.selection.bg_fill, rgba(role::accent::SOFT));
        assert_eq!(v.selection.stroke.color, crate::state::CURRENT);
    }

    /// Hover never grows a control: a target that moves under the pointer
    /// is one a hand misses mid-set.
    #[test]
    fn nothing_grows_under_the_pointer() {
        let v = visuals();
        for w in [&v.widgets.inactive, &v.widgets.hovered, &v.widgets.active] {
            assert_eq!(w.expansion, 0.0);
        }
    }
}
