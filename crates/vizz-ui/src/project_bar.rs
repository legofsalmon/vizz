//! The open show, and the menu that changes it.
//!
//! One chip, at the start of both screens, because "which show am I in"
//! is the question every other control's answer depends on: a pad, a
//! look, a patch and a fader layout all belong to a project, and a
//! program that will not say which one is open is asking you to guess.
//!
//! The menu carries the verbs asked for — new, open, save as, rename,
//! delete — and one line of prose that the verbs cannot say on their own:
//! that there is no save button because there is nothing to press it for.
//! Everything here is written the moment it changes, which predates
//! projects and is not something a live tool should give up. Without that
//! line, "save as…" sitting alone reads as "and if you do not, you lose
//! it", which is both frightening and untrue.

use vizz_design::{feedback, ink, space, surface, text};

/// What the project menu asks the app to do.
///
/// Names rather than indices, and the name the user typed rather than the
/// sanitised one: the app is the thing that knows how a name becomes a
/// directory, and it hands back the name it actually used so the notice
/// can say so. A show typed as `café/bar` lands as `caf__bar`, and
/// silently renaming it without saying would make it unfindable.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct ProjectActions {
    /// Switch to this show.
    pub open: Option<String>,
    /// Start an empty one under this name.
    pub create: Option<String>,
    /// Copy the open one under this name and carry on in the copy.
    pub save_as: Option<String>,
    /// Rename the open one, keeping it open.
    pub rename: Option<String>,
    /// Throw this one away. Never the last one — the menu does not offer
    /// it, and the storage layer refuses it as well.
    pub delete: Option<String>,
}

impl ProjectActions {
    /// Whether anything at all was asked for. The app checks this before
    /// doing any of the work a show change implies — writing the pages
    /// out, moving the pointer, reading a whole show back — none of which
    /// should happen on the overwhelming majority of frames, where the
    /// menu was never opened.
    pub fn any(&self) -> bool {
        self.open.is_some()
            || self.create.is_some()
            || self.save_as.is_some()
            || self.rename.is_some()
            || self.delete.is_some()
    }
}

/// What the menu is asking you to type, if anything.
#[derive(Clone, PartialEq, Eq)]
enum Prompt {
    New(String),
    Copy(String),
    Rename(String),
}

/// Only because egui's temp store asks for it when a value is removed.
/// Nothing ever reads this — a menu holding an empty new-show prompt is
/// a menu that was just closed.
impl Default for Prompt {
    fn default() -> Self {
        Prompt::New(String::new())
    }
}

fn prompt_id() -> egui::Id {
    egui::Id::new("project-prompt")
}

/// Longest name the chip spells out before it starts eliding. Wide enough
/// for a band and a city, narrow enough that it never pushes the output
/// lights off the start of the strip.
const CHIP_CHARS: usize = 22;

fn elide(name: &str) -> String {
    if name.chars().count() <= CHIP_CHARS {
        return name.to_string();
    }
    let head: String = name.chars().take(CHIP_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

/// The chip: the open show's name, and the menu behind it.
///
/// `open` is passed in rather than read here because this runs every
/// frame on both screens. The *list* of shows is a directory read, so it
/// happens inside the popup — where somebody is looking at it — rather
/// than sixty times a second for a menu nobody has opened. The patch
/// loader made the same call for the same reason.
pub fn chip(ui: &mut egui::Ui, open: &str, actions: &mut ProjectActions) {
    let button = egui::Button::new(
        egui::RichText::new(elide(open))
            .size(text::LABEL)
            .color(ink::PRIMARY),
    )
    .fill(surface::WELL)
    .stroke(egui::Stroke::new(1.0, surface::EDGE))
    .min_size(egui::vec2(0.0, 22.0));
    let response = ui
        .add(button)
        .on_hover_text(format!("{open}  ·  the open show — click for new, open and save as"));
    // The same popup the deck chips use, and for the same reason: the
    // menu holds an armed delete and a text field, and egui's default is
    // to close on any click inside — which takes the field away on the
    // first keystroke that lands as a click, and the armed button away
    // between its two presses.
    egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| menu(ui, open, actions));
}

fn menu(ui: &mut egui::Ui, open: &str, actions: &mut ProjectActions) {
    ui.set_min_width(220.0);
    if let Some(prompt) = ui.data(|d| d.get_temp::<Prompt>(prompt_id())) {
        prompt_row(ui, open, prompt, actions);
        return;
    }
    ui.label(
        egui::RichText::new(open)
            .size(text::BODY)
            .color(ink::PRIMARY),
    );
    // The answer to "where is save", given before it is asked.
    ui.label(
        egui::RichText::new("saved as you go")
            .size(text::CAPTION)
            .color(feedback::OK_TEXT),
    );
    ui.separator();

    // Captioned, because with one show on the machine the list is a
    // single row repeating the name in the header, and an unlabelled
    // repeat of a word reads as a mistake rather than as a switcher.
    ui.label(
        egui::RichText::new("OPEN")
            .size(text::MICRO)
            .color(ink::TERTIARY),
    );
    let all = vizz_mod::project::list();
    for name in &all {
        let live = name == open;
        if ui
            .selectable_label(live, egui::RichText::new(name).size(text::LABEL))
            .clicked()
            && !live
        {
            actions.open = Some(name.clone());
            ui.close();
        }
    }
    ui.separator();

    if ui
        .button("new show…")
        .on_hover_text(
            "an empty show: no pages, no pads and none of your saved looks — \
             a show carries its own library so it can be copied whole. The \
             built-in looks are always there, and the built-in set is one \
             right-click away on the deck row's +.",
        )
        .clicked()
    {
        start(ui, Prompt::New(vizz_mod::project::next_name()));
    }
    if ui
        .button("save as…")
        .on_hover_text(
            "copy this show under another name and carry on in the copy. \
             Nothing is lost either way — this show is already saved.",
        )
        .clicked()
    {
        start(ui, Prompt::Copy(format!("{open} copy")));
    }
    if ui.button("rename…").clicked() {
        start(ui, Prompt::Rename(open.to_string()));
    }

    // The last show cannot go: there would be nowhere for the next pad to
    // live and no chip left to click to make one. Hidden rather than
    // disabled, because a greyed item you can never reach is a permanent
    // question about what you did wrong.
    if all.len() > 1 {
        ui.separator();
        if vizz_design::widgets::armed_button(
            ui,
            egui::Id::new("project-delete-armed"),
            0,
            vizz_design::widgets::Armed {
                idle_label: "delete this show",
                armed_label: "delete for good",
                idle_hover: "throw this whole show away — pages, pads, looks and patches (asks once)",
                armed_hover: "everything in this show goes, and there is no undo",
                small: false,
            },
        ) {
            actions.delete = Some(open.to_string());
            ui.close();
        }
    }
}

fn start(ui: &mut egui::Ui, prompt: Prompt) {
    ui.data_mut(|d| {
        d.insert_temp(prompt_id(), prompt);
        d.remove_temp::<bool>(prompt_id().with("focused"));
    });
}

fn stop(ui: &mut egui::Ui) {
    ui.data_mut(|d| {
        d.remove_temp::<Prompt>(prompt_id());
        d.remove_temp::<bool>(prompt_id().with("focused"));
    });
}

/// Typing the name. In the menu rather than in a row under it, unlike the
/// deck rename: a deck's name sits on the pad you are looking at and a
/// popup would cover it, where a show's name is one word at the top of
/// the screen with nothing behind it worth seeing.
fn prompt_row(ui: &mut egui::Ui, open: &str, prompt: Prompt, actions: &mut ProjectActions) {
    let (caption, mut text) = match &prompt {
        Prompt::New(t) => ("name the new show", t.clone()),
        Prompt::Copy(t) => (
            "name the copy — this show stays as it is",
            t.clone(),
        ),
        Prompt::Rename(t) => ("rename this show", t.clone()),
    };
    ui.label(
        egui::RichText::new(caption)
            .size(text::CAPTION)
            .color(ink::SECONDARY),
    );
    let field = ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(200.0)
            .hint_text("show"),
    );
    // Once, on the frame the field appears. Asking every frame re-grabs
    // focus after egui has released it, so `lost_focus` never becomes
    // true and Enter never commits — the same trap the deck rename fell
    // into, with the same fix.
    let focused = prompt_id().with("focused");
    let first = ui.data_mut(|d| {
        let first = !d.get_temp::<bool>(focused).unwrap_or(false);
        d.insert_temp(focused, true);
        first
    });
    if first {
        field.request_focus();
    }
    let typed = text.trim().to_string();
    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.add_space(space::GAP);
    let mut done = false;
    ui.horizontal(|ui| {
        let ok = ui.add_enabled(!typed.is_empty(), egui::Button::new("ok")).clicked();
        if !typed.is_empty() && (ok || enter) {
            match &prompt {
                Prompt::New(_) => actions.create = Some(typed.clone()),
                Prompt::Copy(_) => actions.save_as = Some(typed.clone()),
                // Renaming to the name it already has is not an error and
                // not work: drop it rather than making the app write a
                // notice about a change nobody made.
                Prompt::Rename(_) if typed == open => {}
                Prompt::Rename(_) => actions.rename = Some(typed.clone()),
            }
            done = true;
        }
        if ui.button("cancel").clicked() || escape {
            done = true;
        }
    });
    if done {
        stop(ui);
        ui.close();
    } else {
        let held = match &prompt {
            Prompt::New(_) => Prompt::New(text),
            Prompt::Copy(_) => Prompt::Copy(text),
            Prompt::Rename(_) => Prompt::Rename(text),
        };
        ui.data_mut(|d| d.insert_temp(prompt_id(), held));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// Point config storage at a private directory for the duration of
    /// the guard.
    ///
    /// The menu reads the projects directory when it is open, and a test
    /// that drives it must not read — or create — a real one on whatever
    /// machine is running the suite. `set_var` is process-global, hence
    /// the mutex: nothing else in this crate's test binary touches config
    /// today, and the lock is what keeps that from becoming a flake the
    /// day something does.
    pub(crate) fn scoped_config(tag: &str) -> MutexGuard<'static, ()> {
        let guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("vizz-ui-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the mutex makes this the only thread touching the
        // environment for as long as the guard is held.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        guard
    }

    #[test]
    fn a_long_name_is_elided_rather_than_stretching_the_strip() {
        let long = "The Very Long Show Name That Nobody Would Type";
        let shown = elide(long);
        assert!(shown.chars().count() <= CHIP_CHARS, "{shown}");
        assert!(shown.ends_with('…'));
        assert_eq!(elide("Warehouse"), "Warehouse");
    }

    #[test]
    fn an_empty_action_set_reports_nothing_to_do() {
        assert!(!ProjectActions::default().any());
        assert!(
            ProjectActions {
                open: Some("x".into()),
                ..Default::default()
            }
            .any()
        );
    }
}
