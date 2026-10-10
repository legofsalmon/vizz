//! On-screen notices: the channel every runtime failure was missing.
//!
//! Before this existed, a failed preset save, a rejected file drop, a
//! dying NDI output — all of it went to the log, and a log is a place
//! nobody is looking at 1am with a projector running. The review found
//! eight separate findings that were all this one gap: the app knew
//! something went wrong and had nowhere to say it.
//!
//! Deliberately not a toast framework. A short stack of rows in the top
//! right corner, click to dismiss. Confirmations leave on their own;
//! errors stay until they are clicked away, because the whole point is
//! being seen on the *next* glance at the screen, not the current one —
//! a failure that left before anyone looked did not happen, as far as
//! the room is concerned. That is the shared toast's rule, and so is
//! the arrival: a row rises into place and fades in, and with reduced
//! motion it only fades.

use std::time::{Duration, Instant};

/// How long a row stays. Info is confirmation — it can go quickly.
/// An error stays until it is dismissed.
const INFO_TTL: Duration = vizz_design::motion::NOTICE_TTL;
const ERROR_TTL: Duration = vizz_design::motion::NOTICE_ERROR_TTL;

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
        let mode = vizz_design::motion::mode();
        let mut arriving = false;
        let mut dismissed: Option<usize> = None;
        egui::Area::new(egui::Id::new("notices"))
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                for (i, n) in self.items.iter().enumerate() {
                    // A large thing from an edge: most of the way at once,
                    // then a settle, on the slow duration.
                    let age = n.shown.map_or(0.0, |s| now.duration_since(s).as_secs_f32());
                    let t = vizz_design::motion::ease(
                        vizz_design::motion::EMPHASIZED_ENTER,
                        age / mode.slow,
                    );
                    arriving |= t < 1.0;
                    let (fill, edge, ink) = match n.level {
                        Level::Info => (
                            vizz_design::feedback::OK_BED,
                            vizz_design::surface::EDGE,
                            vizz_design::feedback::ON_OK,
                        ),
                        // The quit prompt's family: red enough to be found
                        // at a glance, dark enough not to strobe the room.
                        Level::Error => (
                            vizz_design::feedback::DANGER_BED,
                            vizz_design::feedback::DANGER_EDGE,
                            vizz_design::feedback::ON_DANGER,
                        ),
                    };
                    ui.add_space((1.0 - t) * mode.offset_medium);
                    let r = ui
                        .scope(|ui| {
                            // From a quarter, not from nothing: the words
                            // are legible in the very first frame, so even
                            // a glance that lands mid-arrival reads them.
                            ui.multiply_opacity(0.25 + 0.75 * t);
                            egui::Frame::NONE
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, edge))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .corner_radius(vizz_design::radius::SHEET)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&n.text).size(13.0).color(ink));
                                // The dismiss, said: a row that only
                                // *could* be clicked away advertised
                                // nothing, and read as something to wait
                                // out.
                                ui.label(
                                    egui::RichText::new("×")
                                        .size(13.0)
                                        .color(ink.gamma_multiply(0.6)),
                                );
                            });
                        })
                        .response
                        })
                        .inner
                        .interact(egui::Sense::click())
                        .on_hover_text("click to dismiss");
                    if r.clicked() {
                        dismissed = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismissed {
            self.items.remove(i);
        }
        if arriving {
            ctx.request_repaint();
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
                egui::Shape::Text(t) => {
                    // The painted glyphs, not the string the galley was
                    // given: an elided label reports its full text
                    // through Galley::text(), so a `contains` check
                    // passes whether or not the words reached the eye.
                    out.extend(t.galley.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)));
                }
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
