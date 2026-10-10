//! The shared widgets: interaction idioms as code, not convention.
//!
//! A design system that stops at colour tables still lets every screen
//! reimplement "ask before destroying" three slightly different ways —
//! which is exactly what had happened. These widgets are the idioms the
//! instrument has settled on, in one place, so using the idiom and
//! matching the idiom are the same act.

use crate::{feedback, motion, text};

/// How an [`armed_button`] presents.
pub struct Armed<'a> {
    /// The resting label ("new", "reset", "x").
    pub idle_label: &'a str,
    /// The armed relabel — the question ("clear?", "reset?", "delete?").
    pub armed_label: &'a str,
    /// Hover at rest. House style ends it with "(asks once)".
    pub idle_hover: &'a str,
    /// Hover while armed — say what the next click destroys.
    pub armed_hover: &'a str,
    /// Draw in the small-button style (for an "x" riding a list row).
    pub small: bool,
}

/// The armed click: the app's one idiom for destructive actions.
///
/// First press relabels the button red for [`motion::ARM_WINDOW`]
/// seconds and does nothing else; a second press inside the window fires
/// (returns `true`); the window lapsing, or the pointer and focus leaving
/// the button, disarms — a press that was not followed through at once
/// was not meant. (Escape does not: in vizz it is the quit key, and a
/// cancel that also asked to quit would be worse than none.) Arming is
/// exclusive per
/// `group`: arming one key disarms any other, so a list of delete
/// buttons can never hold two live triggers at once — the failure mode
/// stays "one extra click", never "a click meant for row A destroying
/// row B".
///
/// The state lives in egui's temp memory under `group`, so callers need
/// no fields, and it survives exactly as long as the UI it belongs to.
pub fn armed_button(ui: &mut egui::Ui, group: egui::Id, key: u64, cfg: Armed<'_>) -> bool {
    let stored: Option<(u64, f64)> = ui.memory_mut(|m| m.data.get_temp(group));
    let now = ui.input(|i| i.time);
    let armed = stored.is_some_and(|(k, t)| k == key && now - t < motion::ARM_WINDOW);

    let button = if armed {
        let label = egui::RichText::new(cfg.armed_label).color(feedback::ON_DANGER_FILL);
        let label = if cfg.small { label.size(text::CAPTION) } else { label };
        egui::Button::new(label).fill(feedback::DANGER_FILL)
    } else if cfg.small {
        egui::Button::new(egui::RichText::new(cfg.idle_label).size(text::CAPTION))
    } else {
        egui::Button::new(cfg.idle_label)
    };
    let button = if cfg.small { button.small() } else { button };

    let response = ui
        .add(button)
        .on_hover_text(if armed { cfg.armed_hover } else { cfg.idle_hover });
    let clicked = response.clicked();
    // `contains_pointer`, not `hovered`: egui reports a press in progress
    // as not hovering, and the second press is exactly that.
    let walked_away = !response.contains_pointer() && !response.has_focus();
    if armed && !clicked && walked_away {
        ui.memory_mut(|m| m.data.remove_temp::<(u64, f64)>(group));
        return false;
    }
    if clicked && armed {
        ui.memory_mut(|m| m.data.remove_temp::<(u64, f64)>(group));
        return true;
    }
    if clicked {
        ui.memory_mut(|m| m.data.insert_temp(group, (key, now)));
    }
    false
}

/// A status dot, painted rather than written.
///
/// egui's default font has no U+25CF, so a text bullet renders as a
/// missing-glyph box — which is exactly what happened the first time a
/// status strip was written here. Filled means live, hollow means not.
pub fn status_dot(ui: &mut egui::Ui, live: bool, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if live {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 4.0, egui::Stroke::new(1.0, color));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a lone armed button, the pointer at `at`; returns
    /// whether it fired and whether it is still armed afterwards.
    fn frame(ctx: &egui::Context, t: f64, events: Vec<egui::Event>) -> (bool, bool) {
        let group = egui::Id::new("test-armed");
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))),
            events,
            time: Some(t),
            ..Default::default()
        });
        let mut fired = false;
        egui::Area::new(egui::Id::new("a"))
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                fired = armed_button(
                    ui,
                    group,
                    7,
                    Armed {
                        idle_label: "reset",
                        armed_label: "reset?",
                        idle_hover: "",
                        armed_hover: "",
                        small: false,
                    },
                );
            });
        let _ = ctx.end_pass();
        let armed = ctx.memory_mut(|m| m.data.get_temp::<(u64, f64)>(group)).is_some();
        (fired, armed)
    }

    fn at(p: egui::Pos2, down: Option<bool>) -> Vec<egui::Event> {
        let mut e = vec![egui::Event::PointerMoved(p)];
        if let Some(pressed) = down {
            e.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        e
    }

    /// A first press that is not followed through at once was not meant:
    /// moving off the button disarms it, and the next press asks again.
    #[test]
    fn walking_away_disarms() {
        let ctx = egui::Context::default();
        let on = egui::pos2(12.0, 10.0);
        let off = egui::pos2(300.0, 150.0);
        frame(&ctx, 0.0, vec![]);
        frame(&ctx, 0.05, at(on, None));
        frame(&ctx, 0.1, at(on, Some(true)));
        assert_eq!(frame(&ctx, 0.2, at(on, Some(false))), (false, true), "the first press should arm");
        assert_eq!(frame(&ctx, 0.3, at(off, None)), (false, false), "leaving should disarm");
        frame(&ctx, 0.4, at(on, Some(true)));
        assert_eq!(frame(&ctx, 0.5, at(on, Some(false))), (false, true), "a press after leaving should only arm");
        frame(&ctx, 0.6, at(on, Some(true)));
        assert!(frame(&ctx, 0.7, at(on, Some(false))).0, "the second press, staying on it, fires");
    }
}
