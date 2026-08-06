//! On-screen notices: the channel every runtime failure was missing.
//!
//! Before this existed, a failed preset save, a rejected file drop, a
//! dying NDI output — all of it went to the log, and a log is a place
//! nobody is looking at 1am with a projector running. The review found
//! eight separate findings that were all this one gap: the app knew
//! something went wrong and had nowhere to say it.
//!
//! Deliberately not a toast framework. A short stack of rows in the top
//! right corner, colour-coded, self-expiring, click to dismiss. Errors
//! outlive infos because the whole point is being seen on the *next*
//! glance at the screen, not the current one.

use std::time::{Duration, Instant};

/// How long a row stays. Info is confirmation — it can go quickly.
/// An error has to survive until the performer next looks over.
const INFO_TTL: Duration = Duration::from_secs(4);
const ERROR_TTL: Duration = Duration::from_secs(15);

/// More than this and the stack is noise; the oldest rows go first.
const MAX_ROWS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Error,
}

#[derive(Debug)]
struct Notice {
    level: Level,
    text: String,
    /// When the notice was first *drawn* — not pushed. The clock starts
    /// on screen: an error raised while the window is occluded or the GUI
    /// is skipping frames must still get its full time in front of the
    /// performer once it finally appears.
    shown: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct Notices {
    items: Vec<Notice>,
}

impl Notices {
    /// Confirmation of something that worked ("saved 'warehouse 2am'").
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Level::Info, text.into());
    }

    /// Something failed and the performer needs to know from across the
    /// room, not from a log file after the show.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Level::Error, text.into());
    }

    fn push(&mut self, level: Level, text: String) {
        // The same message again refreshes the clock instead of stacking:
        // a failure that repeats (an output dying on every retry, a disk
        // that stays full) must read as one persistent fact, not scroll
        // everything else away.
        if let Some(n) = self.items.iter_mut().find(|n| n.text == text) {
            n.shown = None;
            n.level = level;
            return;
        }
        self.items.push(Notice { level, text, shown: None });
        if self.items.len() > MAX_ROWS {
            self.items.remove(0);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Draw the stack and drop what has expired or been clicked.
    ///
    /// Anchored top-right, above everything, drawn whatever else is
    /// hidden — a save failure with the panel closed is precisely the
    /// case this exists for.
    pub fn draw(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.items.retain_mut(|n| {
            let shown = *n.shown.get_or_insert(now);
            now.duration_since(shown)
                < match n.level {
                    Level::Info => INFO_TTL,
                    Level::Error => ERROR_TTL,
                }
        });
        if self.items.is_empty() {
            return;
        }
        let mut dismissed: Option<usize> = None;
        egui::Area::new(egui::Id::new("notices"))
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for (i, n) in self.items.iter().enumerate() {
                    let (fill, ink) = match n.level {
                        Level::Info => (
                            egui::Color32::from_rgb(26, 34, 30),
                            egui::Color32::from_rgb(170, 220, 185),
                        ),
                        // The quit prompt's family: red enough to be found
                        // at a glance, dark enough not to strobe the room.
                        Level::Error => (
                            egui::Color32::from_rgb(110, 38, 34),
                            egui::Color32::from_rgb(255, 236, 232),
                        ),
                    };
                    let r = egui::Frame::NONE
                        .fill(fill)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .corner_radius(5.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&n.text).size(13.0).color(ink));
                        })
                        .response
                        .interact(egui::Sense::click());
                    if r.clicked() {
                        dismissed = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismissed {
            self.items.remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn_text(notices: &mut Notices) -> String {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        // Two passes: egui sizes a fresh Area on the first, draws on the
        // second — the same idiom the grid tests use.
        ctx.begin_pass(input.clone());
        notices.draw(&ctx);
        let _ = ctx.end_pass();
        ctx.begin_pass(input);
        notices.draw(&ctx);
        let out = ctx.end_pass();
        fn walk(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => out.push_str(t.galley.text()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
            out.push(' ');
        }
        let mut text = String::new();
        for s in &out.shapes {
            walk(&s.shape, &mut text);
        }
        text
    }

    /// The reason this module exists: a failure pushed here is on screen.
    #[test]
    fn a_pushed_error_is_actually_drawn() {
        let mut n = Notices::default();
        n.error("could not save preset 'warehouse'");
        let text = drawn_text(&mut n);
        assert!(text.contains("could not save preset"), "not drawn: {text}");
    }

    /// A repeating failure is one persistent row, not a scroll of copies —
    /// an output dying on every 3s retry would otherwise flood the stack.
    #[test]
    fn the_same_message_refreshes_instead_of_stacking() {
        let mut n = Notices::default();
        for _ in 0..10 {
            n.error("output 'ndi:vizz' failed");
        }
        assert_eq!(n.items.len(), 1);
        // And unrelated rows survive alongside it, oldest dropped at cap.
        for i in 0..MAX_ROWS + 2 {
            n.info(format!("note {i}"));
        }
        assert_eq!(n.items.len(), MAX_ROWS);
    }
}
