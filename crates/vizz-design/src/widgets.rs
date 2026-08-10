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
/// (returns `true`); the window lapsing disarms. Arming is exclusive per
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
        let label = egui::RichText::new(cfg.armed_label).color(feedback::ON_DANGER);
        let label = if cfg.small { label.size(text::CAPTION) } else { label };
        egui::Button::new(label).fill(feedback::DANGER_FILL)
    } else if cfg.small {
        egui::Button::new(egui::RichText::new(cfg.idle_label).size(text::CAPTION))
    } else {
        egui::Button::new(cfg.idle_label)
    };
    let button = if cfg.small { button.small() } else { button };

    let clicked = ui
        .add(button)
        .on_hover_text(if armed { cfg.armed_hover } else { cfg.idle_hover })
        .clicked();
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
