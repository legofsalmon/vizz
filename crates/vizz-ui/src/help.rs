//! Help and feedback: the "Send feedback…" form, the crash-report
//! setting, and the one question vizz asks after it has crashed.
//!
//! Like the licence section, nothing here does anything itself. The form
//! hands a draft to the app, which builds the report, queues it and sends
//! it off the render thread; the answer comes back in the next frame's
//! [`HelpView`]. This crate knows nothing about where reports go.

/// What the section shows. Rebuilt by the app each frame the panel is up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HelpView {
    /// This build's version, as the About line prints it.
    pub version: String,
    /// The app holds a licence key, so "include my licence" can be offered.
    pub has_licence: bool,
    /// "Send crash reports automatically".
    pub auto_crash: bool,
    /// Crash reports waiting for a yes.
    pub pending_crashes: usize,
    /// Reports agreed to and not yet taken by the service — sent when a
    /// network is there.
    pub outbox: usize,
    /// The outcome of the last Send, in words: `(is_error, text)`.
    pub message: Option<(bool, String)>,
    /// Bumped by the app each time a draft is accepted, so the form can
    /// clear itself — and only then, so a refused draft is not lost.
    pub sent_revision: u64,
}

/// The four kinds of feedback, in the form's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum FeedbackType {
    #[default]
    Bug,
    Idea,
    Question,
    Praise,
}

impl FeedbackType {
    pub const ALL: [FeedbackType; 4] =
        [FeedbackType::Bug, FeedbackType::Idea, FeedbackType::Question, FeedbackType::Praise];

    pub fn label(self) -> &'static str {
        match self {
            FeedbackType::Bug => "bug",
            FeedbackType::Idea => "idea",
            FeedbackType::Question => "question",
            FeedbackType::Praise => "praise",
        }
    }
}

/// What the person wrote, as it stood when they pressed Send.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeedbackDraft {
    pub kind: FeedbackType,
    pub message: String,
    pub email: String,
    /// "Include my licence so you know who I am". Only ever true when the
    /// box was offered, which is only when the app has a licence.
    pub include_licence: bool,
    /// "OK to post this publicly…". Off unless ticked.
    pub public: bool,
}

/// What the section asks the app to do.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HelpActions {
    /// Send this feedback.
    pub feedback: Option<FeedbackDraft>,
    /// Change "Send crash reports automatically".
    pub set_auto_crash: Option<bool>,
    /// Send the crash reports waiting for a yes.
    pub send_pending: bool,
    /// Throw them away.
    pub discard_pending: bool,
}

/// The answer to the after-a-crash question.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CrashAnswer {
    pub send: bool,
    /// "Always send crash reports" was ticked.
    pub always: bool,
    /// What they were doing, if they said.
    pub note: String,
}

/// Longest message the service takes.
pub const MESSAGE_MAX: usize = 5000;

/// Whether the form may be sent: something written, not too long, and an
/// email that is either absent or plausibly an address. The app checks
/// again, properly; this only keeps the button honest.
pub fn sendable(draft: &FeedbackDraft) -> bool {
    let n = draft.message.trim().chars().count();
    let email = draft.email.trim();
    n > 0
        && n <= MESSAGE_MAX
        && (email.is_empty() || (email.contains('@') && email.contains('.') && !email.contains(' ')))
}

const DRAFT: &str = "help-feedback-draft";
const SEEN: &str = "help-feedback-seen";

/// The section itself.
pub(crate) fn section(ui: &mut egui::Ui, view: &HelpView, actions: &mut HelpActions) {
    ui.label(egui::RichText::new(format!("vizz {}", view.version)).strong());
    ui.horizontal_wrapped(|ui| {
        ui.hyperlink_to("documentation", "https://vizz.letissier.ie/docs");
        ui.hyperlink_to("changelog", "https://vizz.letissier.ie/#changelog");
        ui.hyperlink_to(
            "open-source notices",
            "https://github.com/legofsalmon/vizz/blob/main/THIRD_PARTY_NOTICES.md",
        );
    });
    // The NDI licence asks for this wherever NDI is offered, and the
    // panel is where it is switched on.
    ui.label(
        egui::RichText::new("NDI® is a registered trademark of Vizrt NDI AB.")
            .small()
            .color(vizz_design::ink::TERTIARY),
    );
    ui.hyperlink_to(egui::RichText::new("ndi.video").small(), "https://ndi.video/");
    ui.separator();

    feedback_form(ui, view, actions);
    ui.separator();

    // Crash reports: the setting, and anything waiting on it.
    let mut auto = view.auto_crash;
    if ui
        .checkbox(&mut auto, "Send crash reports automatically")
        .on_hover_text(
            "the version, the OS and the error's backtrace with your user name taken out — \
             never your shows, presets, sources, licence or email",
        )
        .changed()
    {
        actions.set_auto_crash = Some(auto);
    }
    if view.pending_crashes > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!(
                "{} crash report{} waiting",
                view.pending_crashes,
                if view.pending_crashes == 1 { "" } else { "s" }
            ));
            if ui.small_button("send").clicked() {
                actions.send_pending = true;
            }
            if ui.small_button("discard").clicked() {
                actions.discard_pending = true;
            }
        });
    }
    if view.outbox > 0 {
        ui.small(format!(
            "{} report{} will go when there is a network",
            view.outbox,
            if view.outbox == 1 { "" } else { "s" }
        ));
    }
}

fn feedback_form(ui: &mut egui::Ui, view: &HelpView, actions: &mut HelpActions) {
    let id = egui::Id::new(DRAFT);
    let seen_id = egui::Id::new(SEEN);
    let mut draft: FeedbackDraft = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
    // A draft the app accepted since last frame is done with.
    let seen: u64 = ui.data_mut(|d| d.get_temp(seen_id).unwrap_or(view.sent_revision));
    if seen != view.sent_revision {
        draft = FeedbackDraft::default();
    }
    ui.data_mut(|d| d.insert_temp(seen_id, view.sent_revision));

    ui.label(egui::RichText::new("Send feedback…").strong());
    ui.horizontal(|ui| {
        for kind in FeedbackType::ALL {
            ui.selectable_value(&mut draft.kind, kind, kind.label());
        }
    });
    ui.add(
        egui::TextEdit::multiline(&mut draft.message)
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .hint_text(match draft.kind {
                FeedbackType::Bug => "what happened, and what you expected",
                FeedbackType::Idea => "what would make vizz better for you",
                FeedbackType::Question => "ask away",
                FeedbackType::Praise => "what is working for you",
            }),
    );
    ui.add(
        egui::TextEdit::singleline(&mut draft.email)
            .hint_text("email, if you want a reply (optional)")
            .desired_width(f32::INFINITY),
    );
    if view.has_licence {
        ui.checkbox(&mut draft.include_licence, "Include my licence so you know who I am");
    } else {
        draft.include_licence = false;
    }
    ui.checkbox(
        &mut draft.public,
        "OK to post this publicly on the issue tracker, without my name or email",
    );
    let ok = sendable(&draft);
    ui.horizontal(|ui| {
        if ui.add_enabled(ok, egui::Button::new("send")).clicked() {
            actions.feedback = Some(draft.clone());
        }
        let n = draft.message.trim().chars().count();
        if n > MESSAGE_MAX {
            ui.colored_label(vizz_design::feedback::ERR_TEXT, format!("{n}/{MESSAGE_MAX}"));
        }
    });
    if let Some((error, text)) = &view.message {
        let colour = if *error { vizz_design::feedback::ERR_TEXT } else { vizz_design::feedback::OK_TEXT };
        ui.label(egui::RichText::new(text).small().color(colour));
    }
    ui.data_mut(|d| d.insert_temp(id, draft));
}

const NOTE: &str = "crash-prompt-note";
const ALWAYS: &str = "crash-prompt-always";

/// The question, once, after an unclean exit. Drawn over everything and
/// whatever face is up — but only after the app has restored the show,
/// the look and the outputs, so the answer is never between the person
/// and getting the show back. Returns the answer when one is given.
pub(crate) fn crash_prompt(ctx: &egui::Context, count: usize) -> Option<CrashAnswer> {
    let mut answer = None;
    egui::Window::new("vizz closed unexpectedly")
        .id(egui::Id::new("crash-prompt"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(420.0)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.label(
                "Vizz closed unexpectedly last time. Send a crash report to LeTissier Creative Studios?",
            );
            ui.label(
                egui::RichText::new(
                    "Your show, the look that was playing and your outputs have been put back. \
                     The report holds the version, the OS and the error's backtrace with your user \
                     name taken out — never your shows, presets, sources, licence or email.",
                )
                .small()
                .color(vizz_design::ink::TERTIARY),
            );
            if count > 1 {
                ui.small(format!("{count} reports are waiting."));
            }
            let note_id = egui::Id::new(NOTE);
            let always_id = egui::Id::new(ALWAYS);
            let mut note: String = ui.data_mut(|d| d.get_temp(note_id).unwrap_or_default());
            let mut always: bool = ui.data_mut(|d| d.get_temp(always_id).unwrap_or(false));
            ui.add(
                egui::TextEdit::multiline(&mut note)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("what were you doing? (optional)"),
            );
            ui.checkbox(&mut always, "Always send crash reports");
            ui.horizontal(|ui| {
                if ui.button("Send").clicked() {
                    answer = Some(CrashAnswer { send: true, always, note: note.trim().to_string() });
                }
                if ui.button("Don't send").clicked() {
                    answer = Some(CrashAnswer { send: false, always: false, note: String::new() });
                }
            });
            ui.data_mut(|d| {
                d.insert_temp(note_id, note);
                d.insert_temp(always_id, always);
            });
        });
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_send_button_is_only_live_for_a_sendable_draft() {
        let d = |m: &str, e: &str| FeedbackDraft { message: m.into(), email: e.into(), ..Default::default() };
        assert!(!sendable(&d("", "")));
        assert!(!sendable(&d("   ", "")));
        assert!(sendable(&d("hello", "")));
        assert!(sendable(&d("hello", "vj@example.com")));
        assert!(!sendable(&d("hello", "not an email")));
        assert!(!sendable(&d(&"x".repeat(MESSAGE_MAX + 1), "")));
        assert!(sendable(&d(&"x".repeat(MESSAGE_MAX), "")));
    }

    #[test]
    fn a_new_draft_asks_for_nothing_by_default() {
        let d = FeedbackDraft::default();
        assert!(!d.public, "posting publicly must be opted into");
        assert!(!d.include_licence, "the licence must be opted into");
        assert_eq!(d.kind, FeedbackType::Bug);
    }

    /// What `section` paints, as text: the glyphs, not the strings.
    fn painted(view: &HelpView) -> String {
        fn walk(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    out.extend(t.galley.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
            out.push(' ');
        }
        let ctx = egui::Context::default();
        let mut text = String::new();
        for i in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(700.0, 900.0))),
                time: Some(i as f64 * 0.05),
                ..Default::default()
            };
            ctx.begin_pass(input);
            egui::Window::new("help").default_width(640.0).show(&ctx, |ui| {
                let mut actions = HelpActions::default();
                section(ui, view, &mut actions);
            });
            text.clear();
            for s in &ctx.end_pass().shapes {
                walk(&s.shape, &mut text);
            }
        }
        text
    }

    /// The licence box is offered only to a copy that has a licence; the
    /// public box and the crash setting always; the NDI attribution the
    /// SDK licence asks for is always there.
    #[test]
    fn the_form_offers_what_it_should_and_nothing_else() {
        let with = painted(&HelpView { has_licence: true, version: "1.0.0".into(), ..Default::default() });
        assert!(with.contains("Include my licence so you know who I am"), "{with}");
        assert!(with.contains("OK to post this publicly"), "{with}");
        assert!(with.contains("Send crash reports automatically"), "{with}");
        assert!(with.contains("Vizrt NDI AB"), "the NDI attribution is missing: {with}");
        assert!(with.contains("vizz 1.0.0"), "{with}");
        for kind in FeedbackType::ALL {
            assert!(with.contains(kind.label()), "no {} choice: {with}", kind.label());
        }

        let without = painted(&HelpView { pending_crashes: 2, outbox: 1, ..Default::default() });
        assert!(!without.contains("Include my licence"), "offered a licence the app does not have: {without}");
        assert!(without.contains("2 crash reports waiting"), "{without}");
        assert!(without.contains("1 report will go"), "{without}");
    }
}
