//! Sets: a show that ships with the app.
//!
//! A set is a deck per song and a pad per section, plus every look those
//! pads name. It exists because the gap between "vizz runs" and "vizz has
//! a show on it" is an evening of work, and a first launch that opens on
//! sixteen empty pads teaches nothing about what the instrument is for.
//!
//! # Sections, not slots
//!
//! The eight pads are named after the parts of a song — Intro, Build,
//! Break, Drop, Bridge, Peak, Outro, Blackout — and they are the same
//! eight the lighting rig uses. That is the whole point: with both
//! programs following the same column messages, one launch cuts the
//! visuals and the lights together on the same word. A pad called 5 could
//! not do that.
//!
//! # One look per song, expanded
//!
//! The set carries twenty designed looks, one per song: its palette, its
//! geometry, its idiom. What it does not carry is eight separate hand-made
//! variants of each — that would be a hundred and sixty files nobody can
//! check, and the differences between them are not twenty arbitrary
//! decisions but one shape repeated: a bed, a rise, something stripped
//! back, a hit, a hold, the biggest thing in the song, a return to the
//! bed, and an ember.
//!
//! So the shape is written once, in [`SECTIONS`], and each song's look is
//! read through it. The rule is visible and arguable in a way a hundred
//! and sixty blobs would not be, and any pad the operator disagrees with
//! is one `store` away from being theirs.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

use crate::deck::{Book, Deck};
use crate::preset::{Kind, Preset};
use crate::scene::Cell;

/// The designed set, as handed over: twenty songs, each with the palette,
/// the paper and one look.
const ELECTRONIC: &str = include_str!("sets/electronic.json");

/// How many vector layers the registry exposes. Restated here because
/// this crate cannot see the app's parameter table; the enforcement test
/// in `vizz-app` holds the two together.
const LAYERS: usize = 4;

/// A song as the handover describes it.
#[derive(Debug, Clone, Deserialize)]
struct Song {
    n: u32,
    title: String,
    /// 1–5, the song's place in the set's arc. The peaks are at 7, 12 and
    /// 18 and the troughs at 8–9 and 14; this is what keeps a quiet song's
    /// biggest moment smaller than a loud song's quietest one, which is
    /// the difference between a set with a shape and twenty songs in a
    /// row.
    energy: u32,
    values: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, Deserialize)]
struct Designed {
    name: String,
    songs: Vec<Song>,
}

/// What one section does to a song's look.
///
/// The multipliers are against the designed look, and `Drop` is that look
/// exactly — every multiplier 1.0. Everything else is measured from
/// there, which keeps the twenty designs the handover actually drew
/// present in the set rather than approximated by it.
///
/// Opacity carries the intensity, scale opens the picture up as it gets
/// quieter so a break reads as *wide* rather than merely dim, and
/// frequency carries the tension: layers at closer spacing interfere
/// harder, which is how this idiom gets louder without getting brighter.
///
/// That last one is why `Peak` is not simply `Drop` with the opacity
/// pushed. Most of these designs already sit at or near full opacity, so
/// multiplying it only clamps — the two pads would be the same picture.
/// In the lighting they are very nearly the same cue, so the distance
/// between them here is small and it is density, not brightness.
pub struct Section {
    pub name: &'static str,
    /// How many of the song's layers stay on. A stripped section is one
    /// element, not a dimmer version of three — three at low opacity is
    /// mud, and mud is what a break is supposed to clear.
    layers: usize,
    opacity: f32,
    freq: f32,
    scale: f32,
    /// Phase movement per second. Zero is a still frame; the idiom is hard
    /// flat geometry, so this stays small and only really opens up on the
    /// hit.
    drift: f32,
}

/// The shape of a song, in the order the pads are laid out.
///
/// Ranked by intensity the way the lighting is: an ember below everything,
/// then the bed, then the stripped sections, the rises, and the two big
/// ones. `Break` sits under `Build` because it is a clearing rather than a
/// step down, and `Bridge` above it because a bridge keeps moving.
pub const SECTIONS: [Section; 8] = [
    Section { name: "Intro",    layers: 1, opacity: 0.55, freq: 0.85, scale: 1.15, drift: 0.02 },
    Section { name: "Build",    layers: 2, opacity: 0.78, freq: 0.95, scale: 1.05, drift: 0.05 },
    Section { name: "Break",    layers: 1, opacity: 0.62, freq: 0.78, scale: 1.30, drift: 0.03 },
    Section { name: "Drop",     layers: LAYERS, opacity: 1.00, freq: 1.00, scale: 1.00, drift: 0.10 },
    Section { name: "Bridge",   layers: 2, opacity: 0.85, freq: 0.92, scale: 1.08, drift: 0.07 },
    Section { name: "Peak",     layers: LAYERS, opacity: 1.00, freq: 1.12, scale: 0.94, drift: 0.15 },
    Section { name: "Outro",    layers: 2, opacity: 0.60, freq: 0.88, scale: 1.20, drift: 0.04 },
    Section { name: "Blackout", layers: 1, opacity: 0.22, freq: 0.70, scale: 1.45, drift: 0.01 },
];

/// Blends that render black on black paper.
///
/// The paper is black because the picture is projected, and on black,
/// multiply and subtract produce nothing at all: the layer is on, it costs
/// a pass, and the frame is unchanged. A designed look that reaches this
/// is a look someone will spend a soundcheck debugging.
const BLACK_ON_BLACK: [f32; 2] = [1.0, 6.0];
/// What one becomes instead. Add, because on black it is the blend that
/// behaves the way ink on paper is expected to.
const LIT: f32 = 3.0;

/// Registry ranges, restated. See [`LAYERS`].
const FREQ: (f32, f32) = (0.5, 64.0);
const SCALE: (f32, f32) = (0.05, 8.0);
const DRIFT: (f32, f32) = (-2.0, 2.0);

/// A show, ready to be written to disk.
pub struct Set {
    pub name: String,
    /// Every look the decks name, in the order they are played.
    pub presets: Vec<(String, Preset)>,
    pub decks: Vec<Deck>,
}

impl Set {
    /// Names that must resolve for no pad to be dead.
    pub fn preset_names(&self) -> impl Iterator<Item = &str> {
        self.presets.iter().map(|(n, _)| n.as_str())
    }
}

/// The twenty-song electronic set.
pub fn electronic() -> Set {
    let designed: Designed =
        serde_json::from_str(ELECTRONIC).expect("the built-in set is not valid JSON");
    build(&designed)
}

fn build(designed: &Designed) -> Set {
    let mut presets = Vec::new();
    let mut decks = Vec::new();
    for song in &designed.songs {
        let mut deck = Deck::empty(format!("{:02} {}", song.n, song.title));
        for (slot, section) in SECTIONS.iter().enumerate() {
            let name = format!("{:02} {} - {}", song.n, song.title, section.name);
            presets.push((name.clone(), derive(song, section, slot)));
            deck.scenes[slot] = Some(Cell {
                preset: name,
                // The pad says the section, not the song. Every pad on a
                // deck is the same song, so repeating the title sixteen
                // times across a row costs the width that tells you the
                // one thing that differs.
                label: Some(section.name.to_string()),
            });
        }
        decks.push(deck);
    }
    Set { name: designed.name.clone(), presets, decks }
}

/// A song's look as one section plays it.
fn derive(song: &Song, section: &Section, slot: usize) -> Preset {
    let mut values = BTreeMap::new();
    // The paper and the inks are the song's identity and do not move
    // between its sections — a break that changed colour would read as a
    // different song rather than a quieter part of this one.
    for (addr, v) in &song.values {
        if addr.starts_with("/bg/") || addr.starts_with("/pal/") {
            values.insert(addr.clone(), *v);
        }
    }
    let tilt = energy_tilt(song.energy);
    for layer in 1..=LAYERS {
        let p = |name: &str| format!("/l{layer}/{name}");
        let kind = song.values.get(&p("kind")).copied().unwrap_or(0.0);
        // A layer the song does not use, or one this section has stood
        // down. Written explicitly rather than left out: a preset only
        // sets what it names, so an omitted layer keeps whatever the last
        // look put there — which is how a break inherits a peak's third
        // layer and nobody can see why.
        if kind < 0.5 || layer > section.layers {
            values.insert(p("kind"), 0.0);
            values.insert(p("opacity"), 0.0);
            continue;
        }
        let get = |name: &str, fallback: f32| song.values.get(&p(name)).copied().unwrap_or(fallback);

        values.insert(p("kind"), kind);
        values.insert(p("freq"), (get("freq", 8.0) * section.freq).clamp(FREQ.0, FREQ.1));
        // Each section sits at its own place in the cycle, so two sections
        // that happen to land on similar multipliers are still visibly
        // different frames rather than the same one twice.
        values.insert(p("phase"), (get("phase", 0.0) + slot as f32 * 0.11).rem_euclid(1.0));
        // Set on purpose. It is the one parameter the handover predates —
        // it defaults to 0.1, so leaving it out would give every pad in
        // the set a drift nobody chose, on an idiom built from hard flat
        // shapes where unasked-for movement is exactly what spoils it.
        values.insert(p("drift"), (section.drift * tilt).clamp(DRIFT.0, DRIFT.1));
        values.insert(p("duty"), get("duty", 0.5).clamp(0.05, 0.95));
        values.insert(p("sides"), get("sides", 4.0).clamp(2.0, 16.0));
        values.insert(p("inset"), get("inset", 0.5).clamp(0.0, 1.0));
        values.insert(p("fold"), get("fold", 0.0).clamp(0.0, 12.0));
        values.insert(p("invert"), get("invert", 0.0).clamp(0.0, 1.0));
        values.insert(p("x"), get("x", 0.0).clamp(-2.0, 2.0));
        values.insert(p("y"), get("y", 0.0).clamp(-2.0, 2.0));
        values.insert(p("rot"), get("rot", 0.0).clamp(-2.0, 2.0));
        values.insert(p("scale"), (get("scale", 1.0) * section.scale).clamp(SCALE.0, SCALE.1));
        values.insert(p("color"), get("color", 0.0).clamp(0.0, 3.0));
        let blend = get("blend", LIT);
        values.insert(
            p("blend"),
            if BLACK_ON_BLACK.contains(&blend) { LIT } else { blend },
        );
        values.insert(
            p("opacity"),
            (get("opacity", 1.0) * section.opacity * tilt).clamp(0.0, 1.0),
        );
    }
    Preset { values, source: Some("electronic set".into()) }
}

/// How far a song of this energy is allowed to open up.
///
/// A quiet song's peak has to stay under a loud song's, or the arc the set
/// is built around — peaks at 7, 12 and 18, troughs at 8–9 and 14 — is
/// flattened into twenty songs that all reach the same place. The spread
/// is deliberately narrow: the palette and the geometry already say which
/// song this is, and dimming a quiet song into invisibility would lose the
/// thing it was designed for.
fn energy_tilt(energy: u32) -> f32 {
    0.72 + energy.clamp(1, 5) as f32 * 0.056
}

/// Write a set's looks to the library and return the book its decks make.
///
/// The looks go down first. A deck names its looks, so a book installed
/// over a library that does not have them yet is a row of dead pads —
/// briefly, but during the one launch a new user is deciding what this
/// program is.
pub fn install(set: &Set) -> Result<Book> {
    for (name, preset) in &set.presets {
        crate::preset::save_kind(Kind::Look, name, preset)?;
    }
    let mut book = Book::default();
    book.replace(set.decks.clone());
    Ok(book)
}

/// Whether this looks like a machine that has never had a show on it.
///
/// Deliberately strict. Installing over somebody's set would be
/// unforgivable, and the cost of being wrong in the other direction is
/// that a user who cleared every pad has to ask for the set from the menu.
pub fn is_fresh_install(book: &Book, saved: bool) -> bool {
    !saved && book.len() == 1 && book.decks()[0].is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> Set {
        electronic()
    }

    /// Twenty songs, eight sections, and a look behind every pad. A deck
    /// naming a look that was never written is a pad that does nothing
    /// when pressed, and the failure is invisible until someone presses
    /// it.
    #[test]
    fn every_pad_in_the_set_names_a_look_that_exists() {
        let set = set();
        assert_eq!(set.decks.len(), 20, "the set is not twenty songs");
        let names: std::collections::BTreeSet<&str> = set.preset_names().collect();
        assert_eq!(names.len(), 160, "twenty songs of eight sections is a hundred and sixty");

        let mut pads = 0;
        for deck in &set.decks {
            for (slot, cell) in deck.scenes.iter().enumerate() {
                match (slot < SECTIONS.len(), cell) {
                    (true, Some(cell)) => {
                        assert!(
                            names.contains(cell.preset.as_str()),
                            "{} pad {slot} names {:?}, which is not in the set",
                            deck.name,
                            cell.preset
                        );
                        assert_eq!(cell.display(), SECTIONS[slot].name);
                        pads += 1;
                    }
                    (true, None) => panic!("{} has nothing on pad {slot}", deck.name),
                    // Past the sections, the deck is empty on purpose:
                    // sixteen pads, eight of them a song.
                    (false, cell) => assert!(cell.is_none(), "{} pad {slot} is not empty", deck.name),
                }
            }
            assert_eq!(deck.scenes.len(), crate::scene::SLOTS);
        }
        assert_eq!(pads, 160);
    }

    /// The pads are named after the parts of a song, in the order the
    /// lighting rig uses them. If these drift apart, one launch cuts the
    /// visuals and the lights to different places — which is the entire
    /// thing the shared columns are for.
    #[test]
    fn the_sections_are_the_ones_the_lighting_rig_uses() {
        assert_eq!(
            SECTIONS.map(|s| s.name),
            ["Intro", "Build", "Break", "Drop", "Bridge", "Peak", "Outro", "Blackout"]
        );
    }

    /// Nothing in the set can render black on black.
    ///
    /// The paper is black because the picture is projected, so multiply
    /// and subtract produce a layer that is on, costs a pass and shows
    /// nothing. A designed look reaching this is a soundcheck spent
    /// debugging a layer that was never going to appear.
    #[test]
    fn no_layer_in_the_set_is_invisible_against_the_paper() {
        for (name, preset) in &set().presets {
            for layer in 1..=LAYERS {
                let on = preset
                    .values
                    .get(&format!("/l{layer}/kind"))
                    .is_some_and(|k| *k >= 0.5);
                if !on {
                    continue;
                }
                let blend = preset.values[&format!("/l{layer}/blend")];
                assert!(
                    !BLACK_ON_BLACK.contains(&blend),
                    "{name} layer {layer} blends {blend}, which renders black on black paper"
                );
            }
            for channel in ["red", "green", "blue"] {
                assert_eq!(
                    preset.values[&format!("/bg/{channel}")], 0.0,
                    "{name} drifted off black paper"
                );
            }
        }
    }

    /// Every layer that is on writes every one of its parameters.
    ///
    /// A preset only sets what it names, so an omitted parameter keeps
    /// whatever the previous look left there. On a set played by cutting
    /// between pads, that means a break inheriting a peak's fold or a
    /// song inheriting the last one's rotation — a look that is right the
    /// first time it is fired and wrong every time after.
    #[test]
    fn an_enabled_layer_leaves_nothing_to_the_look_before_it() {
        const PER_LAYER: [&str; 16] = [
            "kind", "freq", "phase", "drift", "duty", "sides", "inset", "fold", "invert", "x",
            "y", "rot", "scale", "color", "blend", "opacity",
        ];
        for (name, preset) in &set().presets {
            for layer in 1..=LAYERS {
                let kind = preset.values.get(&format!("/l{layer}/kind"));
                assert!(kind.is_some(), "{name} never says whether layer {layer} is on");
                if *kind.unwrap() < 0.5 {
                    // An off layer says so, and says it is silent — the
                    // two together are what stop it inheriting.
                    assert_eq!(preset.values[&format!("/l{layer}/opacity")], 0.0);
                    continue;
                }
                for p in PER_LAYER {
                    assert!(
                        preset.values.contains_key(&format!("/l{layer}/{p}")),
                        "{name} layer {layer} never sets {p}"
                    );
                }
            }
        }
    }

    /// A song's sections are ranked the way its lighting is: an ember
    /// under the bed, the bed under the rises, and the peak over
    /// everything. Without this the eight pads are eight arbitrary looks
    /// and pressing them in order is not a performance.
    #[test]
    fn the_sections_of_a_song_are_ranked_by_intensity() {
        let set = set();
        // Sirens, the summit, where every section is at its most spread.
        let song: Vec<&(String, Preset)> = set
            .presets
            .iter()
            .filter(|(n, _)| n.starts_with("18 Sirens"))
            .collect();
        assert_eq!(song.len(), 8);
        let lit = |name: &str| {
            let (_, p) = song.iter().find(|(n, _)| n.ends_with(name)).unwrap();
            p.values["/l1/opacity"]
        };
        assert!(lit("Blackout") < lit("Intro"), "the ember is brighter than the bed");
        assert!(lit("Intro") < lit("Build"), "the build does not rise");
        assert!(lit("Break") < lit("Build"), "the break does not clear");
        assert!(lit("Build") < lit("Bridge"));
        assert!(lit("Bridge") < lit("Drop"));
        assert!(lit("Drop") <= lit("Peak"), "the peak is not the biggest thing in the song");
        assert!(lit("Outro") < lit("Bridge"), "the outro does not come back down");
    }

    /// A stripped section is one element, not three at low opacity —
    /// three dim layers is mud, and clearing the mud is what a break is
    /// for.
    #[test]
    fn a_break_stands_the_other_layers_down() {
        let set = set();
        let on = |name: &str| {
            let (_, p) = set.presets.iter().find(|(n, _)| n == name).unwrap();
            (1..=LAYERS)
                .filter(|l| p.values[&format!("/l{l}/kind")] >= 0.5)
                .count()
        };
        // Open Water runs three layers at its peak.
        assert_eq!(on("12 Open Water - Peak"), 3);
        assert_eq!(on("12 Open Water - Break"), 1, "the break kept its other layers");
        assert_eq!(on("12 Open Water - Intro"), 1);
        assert_eq!(on("12 Open Water - Blackout"), 1);
        assert_eq!(on("12 Open Water - Build"), 2);
    }

    /// A quiet song's biggest moment stays under a loud song's, or the
    /// set's arc — peaks at 7, 12 and 18, troughs at 8–9 and 14 — is
    /// twenty songs that all reach the same place.
    #[test]
    fn the_arc_survives_into_the_pads() {
        let set = set();
        let peak = |prefix: &str| {
            let (_, p) = set
                .presets
                .iter()
                .find(|(n, _)| n.starts_with(prefix) && n.ends_with("Peak"))
                .unwrap_or_else(|| panic!("no peak for {prefix}"));
            p.values["/l1/opacity"]
        };
        // 14 Vanishing Point is the genuine bottom; 18 Sirens the summit.
        assert!(
            peak("14 Vanishing Point") < peak("18 Sirens"),
            "the trough's peak is not smaller than the summit's"
        );
        assert!(peak("01 Still Air") < peak("07 Flare Path"));
        assert!(peak("09 Low Ceiling") < peak("12 Open Water"));
    }

    /// The palette is the song and does not move between its sections. A
    /// break that changed colour would read as a different song rather
    /// than a quieter part of this one.
    #[test]
    fn a_songs_colour_is_the_same_in_every_section() {
        let set = set();
        let song: Vec<&(String, Preset)> =
            set.presets.iter().filter(|(n, _)| n.starts_with("05 Verdigris")).collect();
        let first = &song[0].1;
        for (name, preset) in &song[1..] {
            for i in 0..4 {
                for c in ["r", "g", "b"] {
                    let addr = format!("/pal/{i}/{c}");
                    assert_eq!(
                        preset.values[&addr], first.values[&addr],
                        "{name} moved {addr} away from the song's palette"
                    );
                }
            }
        }
    }

    /// The `Drop` pad is the designed look, exactly.
    ///
    /// The set expands the twenty designs; it does not replace them. One
    /// pad per song therefore has to *be* the drawing, or the handover's
    /// work only survives as an approximation of itself — and there would
    /// be nothing to check a later change to [`SECTIONS`] against.
    ///
    /// Three things still differ, on purpose: opacity carries the song's
    /// place in the arc, phase separates the sections from each other, and
    /// drift is a parameter the handover predates.
    #[test]
    fn the_drop_pad_is_the_designed_look_exactly() {
        let designed: Designed = serde_json::from_str(ELECTRONIC).unwrap();
        let set = set();
        let mut compared = 0;
        for song in &designed.songs {
            let want_name = format!("{:02} {} - Drop", song.n, song.title);
            let (_, drop) = set.presets.iter().find(|(n, _)| *n == want_name).unwrap();
            for (addr, want) in &song.values {
                if addr.ends_with("/opacity") || addr.ends_with("/phase") {
                    continue;
                }
                assert_eq!(
                    drop.values.get(addr),
                    Some(want),
                    "{want_name} changed {addr} — the drop is not the drawing"
                );
                compared += 1;
            }
        }
        assert!(compared > 500, "only compared {compared} values");
    }

    /// Installing puts every look on disk under the name its pad uses.
    ///
    /// The one failure that matters here is silent: a deck names a look by
    /// string, so a preset saved under a name the sanitiser changed — or
    /// not saved at all — leaves a pad that looks filled and does nothing
    /// when pressed. Nothing else in the crate would notice.
    #[test]
    fn installing_leaves_every_pad_pointing_at_a_look_on_disk() {
        let (_guard, _dir) = crate::test_env::scoped("sets-install");
        let set = electronic();
        let book = install(&set).expect("install failed");

        assert_eq!(book.len(), 20);
        assert_eq!(book.active(), 0, "installing did not land on the first song");

        let mut library = crate::preset::Library::new();
        library.refresh();
        for deck in book.decks() {
            for cell in deck.scenes.iter().flatten() {
                assert!(
                    library.has(Kind::Look, &cell.preset),
                    "{} names {:?}, which did not survive being saved",
                    deck.name,
                    cell.preset
                );
                // And it loads back as a look, not merely as a filename.
                let loaded = crate::preset::by_name(&cell.preset)
                    .unwrap_or_else(|| panic!("{:?} would not load", cell.preset));
                assert!(
                    loaded.values.contains_key("/l1/kind"),
                    "{:?} came back without its first layer",
                    cell.preset
                );
            }
        }
    }

    /// A set is only installed over a machine that has never had a show on
    /// it. Installing over somebody's own decks would be unforgivable, and
    /// the test is deliberately the strict one.
    #[test]
    fn a_show_that_already_exists_is_never_installed_over() {
        let fresh = Book::default();
        assert!(is_fresh_install(&fresh, false), "a first run was not recognised");
        // A deck list on disk is a show, however empty it looks.
        assert!(!is_fresh_install(&fresh, true));

        let mut used = Book::default();
        let (mut scenes, gravity) = (crate::scene::Grid::new(), crate::scene::Grid::for_kind(Kind::Gravity));
        scenes.assign(0, "theirs");
        used.store(&scenes, &gravity);
        assert!(!is_fresh_install(&used, false), "a pad with a look on it was not noticed");
    }
}
