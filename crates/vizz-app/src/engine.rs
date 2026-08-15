//! Frame engine: everything one frame needs that isn't the GPU itself.
//! Shared verbatim between windowed and headless modes so a benchmark run
//! measures the same code a live set executes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use vizz_audio::{AudioEngine, BAND_COUNT};
use vizz_health::{HealthConfig, HealthMonitor, HealthSnapshot};
use vizz_mod::{AudioLevels, ModEngine};
use vizz_params::ParamSnapshot;
use vizz_render::camera::Camera;
use vizz_render::particles::Uniforms;
use vizz_render::post::PostUniforms;
use vizz_render::room::RoomUniforms;

use crate::params::AppParams;

pub struct FrameEngine {
    params: Arc<AppParams>,
    pub snapshot: ParamSnapshot,
    pub health: HealthMonitor,
    /// LFOs and the beat clock. Render-thread-owned: it ticks here and the
    /// panel (drawn on this thread) edits it directly.
    pub modulation: ModEngine,
    /// Audio capture and analysis. Always present — when no device could
    /// be opened it simply reports zeros, so nothing downstream needs a
    /// branch for "no audio".
    pub audio: AudioEngine,
    /// Scratch for this frame's band envelopes, so the tick borrows a
    /// slice rather than allocating.
    bands: [f32; BAND_COUNT],
    /// Visual time, pre-integrated so `/particles/speed` changes modulate
    /// the rate without jumping the phase.
    ///
    /// `f64` because this is a running sum with a small increment. In
    /// `f32` the increment starts being lost into the accumulator's own
    /// rounding as the total grows: with a 60 Hz frame and speed 1, the
    /// step is about a sixtieth of a second against a value whose spacing
    /// has widened to a comparable size after a few hours, and additions
    /// go missing entirely at around six days of running. An installation
    /// left up overnight is inside the first of those.
    ///
    /// This fixes the accumulator, not the shader. What is handed to the
    /// GPU is still an `f32` of the same magnitude, so its own resolution
    /// at large times is unchanged. Wrapping it would fix that too, and is
    /// not available: the per-particle spin rate is a continuous range, so
    /// the field has no period to wrap at and any wrap would jump it. That
    /// is a look change rather than a bug fix, so it is not made here.
    vis_time: f64,
    last_frame: Option<Instant>,
    last_log: Instant,
    /// Last `/preset/recall` index acted on. Presets fire on *change*, so
    /// a controller parked on an index does not re-apply it every frame
    /// and fight whatever you are adjusting by hand.
    last_preset: Option<usize>,
    /// The scene grid and the blend between its cells. Lives here rather
    /// than in the UI because it writes parameter targets every frame and
    /// has to keep doing so with the panel hidden.
    pub grid: vizz_mod::scene::Grid,
    /// Last `/scene/fire` slot acted on, edge-triggered like recall.
    last_scene: Option<usize>,
    /// A zero-second scene change landed this frame and the smoothing
    /// has to be skipped once, or "cut" fades like everything else.
    cut_pending: bool,
    /// Keep the gravity sequencer on the scene sequencer's settings.
    /// Set by the app from the saved preference and the deck's toggle.
    pub autopilot_lock: bool,
    /// The gravity layer's own grid, sequencing gravity presets on its own
    /// clock. A second instance of the same machine rather than a special
    /// case: the grid already takes its preset lookup as a parameter, so
    /// pointing one at a different library is all a second layer needs.
    pub gravity_grid: vizz_mod::scene::Grid,
    last_gravity: Option<usize>,
    /// The pages of pads, and which one is live.
    ///
    /// Here rather than in the app for the same reason the grids are: a
    /// page turn writes both grids' cells, and the thing that owns the
    /// grids has to be the thing that swaps them or there are two writers
    /// and one of them is a frame behind.
    pub decks: vizz_mod::deck::Book,
    /// Last `/deck/select` index acted on, edge-triggered like recall.
    ///
    /// The authoritative current deck is the book's, not this. A MIDI
    /// trigger drives its parameter back to rest on release — that is what
    /// makes a second press of the same pad work — so the parameter reads
    /// 0 while deck 3 is live, and anything asking it "which deck am I on"
    /// gets the wrong answer the moment a finger comes off a button.
    last_deck: Option<usize>,
    /// What Resolume's column launches arrive through. Shared with the
    /// OSC listener; see [`vizz_osc::ColumnSync`].
    columns: Arc<vizz_osc::ColumnSync>,
    last_column: Option<usize>,
    /// The listener's fire count as of the last frame. A column relaunched
    /// while it is already showing has to fire again, and the slot number
    /// alone cannot say that happened.
    last_column_fires: u32,
    /// A page turn happened inside the frame and the set list is worth
    /// writing. Collected here rather than returned, because the deck can
    /// change from a controller or an OSC message with nothing on screen
    /// having been touched, and the caller cannot know to ask.
    decks_dirty: bool,
}

pub struct FrameInputs {
    pub uniforms: Uniforms,
    pub post: PostUniforms,
    pub room: RoomUniforms,
    /// Skip the room pass entirely when it is dark — it is off by default,
    /// and drawing invisible lines every frame is wasted work.
    pub room_visible: bool,
    pub count: u32,
    /// What an empty frame looks like, alpha included. At alpha 0 the
    /// field is delivered on a transparent background so vizz can be a
    /// layer in a mixer rather than the whole picture.
    pub background: wgpu::Color,
    /// The vector layer stack, packed for the shader. The caller fills
    /// the render height into `bg[3]` before drawing — the engine does
    /// not know the target size, and the shader derives its analytic
    /// pixel footprint from it.
    pub vector: vizz_render::vector::StackU,
    /// Whether any layer is on. When false the vector pass is skipped
    /// entirely and the frame is byte-identical to one rendered before
    /// vector layers existed — the guarantee that makes shipping an
    /// experimental renderer inside the live instrument tolerable.
    pub vector_active: bool,
    /// True when `/vec/place` says "print": the stack draws after the
    /// post chain instead of into it.
    pub vector_print: bool,
}

impl FrameEngine {
    pub fn new(params: Arc<AppParams>, audio: AudioEngine) -> Self {
        Self {
            snapshot: ParamSnapshot::new(&params.registry),
            params,
            health: HealthMonitor::new(HealthConfig::default()),
            // Whatever was on the rack last time. Every other piece of
            // user state comes back on the next launch; this one used to
            // be thrown away, routes and all.
            modulation: vizz_mod::library::load_session()
                .unwrap_or_else(ModEngine::with_defaults),
            audio,
            bands: [0.0; BAND_COUNT],
            vis_time: 0.0,
            last_frame: None,
            last_log: Instant::now(),
            last_preset: None,
            grid: vizz_mod::scene::Grid::new(),
            last_scene: None,
            cut_pending: false,
            autopilot_lock: false,
            gravity_grid: vizz_mod::scene::Grid::for_kind(vizz_mod::preset::Kind::Gravity),
            last_gravity: None,
            decks: vizz_mod::deck::Book::default(),
            last_deck: None,
            columns: Arc::new(vizz_osc::ColumnSync::default()),
            last_column: None,
            last_column_fires: 0,
            decks_dirty: false,
        }
    }

    /// Whether the set list has changed since this was last asked, and
    /// should be written to disk. Taken rather than read, so one save
    /// answers one change.
    pub fn take_decks_dirty(&mut self) -> bool {
        std::mem::take(&mut self.decks_dirty)
    }

    /// Share the listener's column state.
    ///
    /// Adopted rather than passed to the constructor because the listener
    /// is bound before the window exists and the engine is built after it.
    /// A engine that never adopts one keeps the standalone it was built
    /// with, which is inert and is what every test and the headless path
    /// run on.
    pub fn adopt_column_sync(&mut self, columns: Arc<vizz_osc::ColumnSync>) {
        // Origin first, then the switch. The listener starts before the
        // book is read, so until this runs it has no idea which of
        // Resolume's columns the live page covers — and a column arriving
        // in that window would be measured against the wrong stretch. So
        // following is turned on *here*, once both halves are known,
        // rather than at the bind: the cost is that a column launched
        // during the second the window takes to open is ignored, which is
        // the right way round.
        columns
            .origin
            .store(self.decks.origin(), std::sync::atomic::Ordering::Relaxed);
        columns.enabled.store(
            crate::settings::load().follow_columns,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.columns = columns;
        // Start from the listener's count, not from zero. It has been
        // running since before the window opened, so a fresh latch would
        // read every column it accepted in that time as one relaunch. The
        // first frame's decision is then made by the column *value* alone,
        // which is what catches vizz up to whatever Arena is showing.
        self.last_column_fires = self
            .columns
            .fires
            .load(std::sync::atomic::Ordering::Acquire);
    }

    /// Whether Resolume's column launches are being followed.
    pub fn follow_columns(&self) -> bool {
        self.columns
            .enabled
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Start or stop following Resolume's columns.
    pub fn set_follow_columns(&mut self, follow: bool) {
        self.columns
            .enabled
            .store(follow, std::sync::atomic::Ordering::Relaxed);
    }

    /// Adopt the deck book loaded from disk and put its live page on the
    /// grids.
    ///
    /// Called after both grids have been adopted, because the book's own
    /// idea of the live page is a mirror of what is already in the grid
    /// files — see `vizz_mod::deck::load`.
    pub fn adopt_decks(&mut self, decks: vizz_mod::deck::Book) {
        self.decks = decks;
        self.columns
            .origin
            .store(self.decks.origin(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Turn to a page, from the UI rather than through the parameter.
    ///
    /// Returns whether anything moved, so the caller knows whether the
    /// set list is worth writing to disk.
    pub fn switch_deck(&mut self, index: usize) -> bool {
        if !self
            .decks
            .switch(index, &mut self.grid, &mut self.gravity_grid)
        {
            return false;
        }
        self.after_deck_change();
        true
    }

    /// Everything a page turn implies beyond the cells themselves.
    ///
    /// Public because a page can also arrive by being created, copied or
    /// deleted, and those go through the book directly.
    pub fn after_deck_change(&mut self) {
        // Both fire parameters back to rest, latches with them.
        //
        // Not one or the other. Clearing the latch alone would leave the
        // parameter holding the slot fired on the old page, so the very
        // next frame would read a change and fire that number on the new
        // one — turning a page would play a scene, which is the one thing
        // a page turn must never do. Leaving both alone instead makes the
        // pad you are most likely to reach for dead: arriving from pad 3
        // of the old deck, pad 3 of the new one is not a change and does
        // nothing at all.
        //
        // Rest is the same trick a MIDI trigger uses on release, and for
        // the same reason: passing through zero is what makes the next
        // press of any pad, including the one just pressed, land.
        let reg = &self.params.registry;
        reg.set(self.params.scene_fire, 0.0);
        reg.set(self.params.gravity_fire, 0.0);
        self.last_scene = Some(0);
        self.last_gravity = Some(0);
        // And the page control itself, to the page that is now live.
        //
        // A page can arrive by being created, copied or deleted, and those
        // reach the book directly rather than through the parameter — so
        // without this the parameter goes on naming the chip that was live
        // before the "+", and clicking that chip writes the number it
        // already holds. No edge, no turn, and the chip is dead until you
        // click a different one first.
        //
        // Both halves again. Moving the latch alone would leave the
        // parameter naming the old page, and the next frame would read
        // that as a change and turn straight back off the page just made.
        let live = self.decks.active() as f32 + 1.0;
        reg.set(self.params.deck_select, live);
        self.last_deck = Some(live as usize);
        // The column path needs no such reset: it re-arms off the
        // listener's counter, so a Resolume column relaunched after a page
        // turn lands on its own.
        self.refresh_column_origin();
    }

    /// Tell the listener which of Resolume's columns the live page covers.
    pub fn refresh_column_origin(&mut self) {
        self.columns
            .origin
            .store(self.decks.origin(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Adopt a grid loaded from disk, pushing its stored transition
    /// settings out to the parameters that drive them.
    ///
    /// Those settings live in two places by necessity — in the grid,
    /// because a `Grid` has to be usable and testable on its own, and in
    /// the parameter store, because that is what gives them OSC, MIDI
    /// learn and a fader. The parameters are the authority from here on,
    /// so the saved values have to be written *into* them at load or they
    /// would be silently replaced by the defaults on the first frame.
    pub fn adopt_grid(&mut self, grid: vizz_mod::scene::Grid) {
        let reg = &self.params.registry;
        reg.set(self.params.scene_time, grid.duration);
        reg.set(
            self.params.scene_curve,
            vizz_mod::scene::Curve::ALL
                .iter()
                .position(|c| *c == grid.curve)
                .unwrap_or(1) as f32,
        );
        reg.set(
            self.params.scene_auto,
            if grid.autopilot.enabled { 1.0 } else { 0.0 },
        );
        reg.set(self.params.scene_bars, grid.autopilot.bars);
        self.grid = grid;
    }

    /// As [`Self::adopt_grid`], for the gravity layer.
    pub fn adopt_gravity_grid(&mut self, grid: vizz_mod::scene::Grid) {
        let reg = &self.params.registry;
        reg.set(self.params.gravity_time, grid.duration);
        reg.set(
            self.params.gravity_curve,
            vizz_mod::scene::Curve::ALL
                .iter()
                .position(|c| *c == grid.curve)
                .unwrap_or(1) as f32,
        );
        reg.set(
            self.params.gravity_auto,
            if grid.autopilot.enabled { 1.0 } else { 0.0 },
        );
        reg.set(self.params.gravity_bars, grid.autopilot.bars);
        self.gravity_grid = grid;
    }

    /// Turn the page when `/deck/select` has moved, and fire a column when
    /// one has arrived.
    ///
    /// Both before [`FrameEngine::tick_grid`], so a page turn and a pad
    /// press landing on the same frame happen in that order — the pad
    /// belongs to the deck that was selected, not the one being left.
    ///
    /// Returns whether the set list changed and is worth saving. A page
    /// turn is the only gesture here that writes cells.
    fn tick_decks(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        let p = Arc::clone(&self.params);
        let reg = &p.registry;

        let mut turned = false;
        let deck = reg.target(p.deck_select).round().max(0.0) as usize;
        if self.last_deck != Some(deck) {
            self.last_deck = Some(deck);
            if let Some(index) = deck.checked_sub(1) {
                // An index past the end is silence rather than a clamp, and
                // the edge is recorded either way: a fader swept across a
                // bank of eight buttons on a show with three decks must not
                // retry the same missing deck sixty times a second.
                turned = self.switch_deck(index);
            }
        }

        // A column launch is a scene pad and a gravity pad of the same
        // number, fired together — which is what a column *is* in the
        // program this follows.
        let fires = self.columns.fires.load(Ordering::Acquire);
        let relaunched = fires != self.last_column_fires;
        self.last_column_fires = fires;
        let column = reg.target(p.column_fire).round().max(0.0) as usize;
        if relaunched || self.last_column != Some(column) {
            self.last_column = Some(column);
            if column > 0 {
                // Through the fire parameters rather than into the grids,
                // so a column, a pad click and a MIDI note are one path.
                // The latches go with it: relaunching the column already
                // showing is a deliberate re-trigger and has to land, and
                // the value alone cannot say a second launch happened.
                self.last_scene = None;
                self.last_gravity = None;
                reg.set(p.scene_fire, column as f32);
                reg.set(p.gravity_fire, column as f32);
            }
        }
        turned
    }

    /// Fire a scene when `/scene/fire` has moved, then advance the blend.
    ///
    /// Edge-triggered for the same reason recall is: a pad controller
    /// holding a slot down must not restart the transition every frame.
    /// Slot 0 is "nothing selected" and the grid runs from 1, so a fresh
    /// start cannot fire cell 0 over your defaults.
    ///
    /// The transition settings are read from the parameters every frame,
    /// so a knob or an OSC message changes them mid-set. `duration` is
    /// only read *between* transitions — changing the blend time while one
    /// is running would make the transition already in flight jump.
    fn tick_grid(&mut self, dt: f32) {
        use vizz_mod::scene::Curve;
        let p = Arc::clone(&self.params);
        let reg = &p.registry;

        if self.grid.in_flight().is_none() {
            self.grid.duration = reg.target(p.scene_time);
        }
        let curve = reg.target(p.scene_curve).round().max(0.0) as usize;
        self.grid.curve = Curve::ALL.get(curve).copied().unwrap_or_default();
        self.grid.autopilot.enabled = reg.target(p.scene_auto) >= 0.5;
        self.grid.autopilot.bars = reg.target(p.scene_bars);

        // Scenes name presets rather than carrying copies, so firing one
        // resolves through the library here. Built-ins and saved presets
        // are equally addressable, which is what lets a set be prepared
        // from either.
        let presets = |name: &str| vizz_mod::preset::by_name(name);
        let slot = reg.target(p.scene_fire).round().max(0.0) as usize;
        if self.last_scene != Some(slot) {
            self.last_scene = Some(slot);
            if let Some(index) = slot.checked_sub(1) {
                self.grid.fire(index, reg, &presets);
                // A zero-second blend is a cut, and has to reach the
                // picture as one. The grid writes its targets instantly,
                // but every parameter then crossed over its own
                // smoothing constant — so "cut" faded, and the one
                // control whose whole job is to be instant was the one
                // that could not be. Marked here and honoured after the
                // slew runs, which is the only place it can be undone.
                if self.grid.duration <= f32::EPSILON {
                    self.cut_pending = true;
                }
            }
        }
        self.grid
            .tick(dt, self.modulation.clock.beats, reg, &presets);

        // The gravity layer, on its own transport and its own library.
        // Independent all the way down: a look firing cannot disturb the
        // wells and a well firing cannot disturb the look, which is the
        // whole reason the two are separate grids rather than one.
        let gravity_presets =
            |name: &str| vizz_mod::preset::load_kind(vizz_mod::preset::Kind::Gravity, name).ok();
        // Locked: the gravity sequencer takes the scene sequencer's rate
        // and the shape of its changes, written through the parameters
        // so the panel, OSC and MIDI all see the same values rather than
        // showing stale ones beside a grid quietly doing something else.
        //
        // Rate and shape only. *Which pads each grid holds* is the whole
        // reason there are two, so the lock never touches that — it stops
        // them drifting apart in time, not in content.
        if self.autopilot_lock {
            reg.set(p.gravity_time, reg.target(p.scene_time));
            reg.set(p.gravity_curve, reg.target(p.scene_curve));
            reg.set(p.gravity_auto, reg.target(p.scene_auto));
            reg.set(p.gravity_bars, reg.target(p.scene_bars));
        }
        if self.gravity_grid.in_flight().is_none() {
            self.gravity_grid.duration = reg.target(p.gravity_time);
        }
        let gcurve = reg.target(p.gravity_curve).round().max(0.0) as usize;
        self.gravity_grid.curve = Curve::ALL.get(gcurve).copied().unwrap_or_default();
        self.gravity_grid.autopilot.enabled = reg.target(p.gravity_auto) >= 0.5;
        self.gravity_grid.autopilot.bars = reg.target(p.gravity_bars);
        let gslot = reg.target(p.gravity_fire).round().max(0.0) as usize;
        if self.last_gravity != Some(gslot) {
            self.last_gravity = Some(gslot);
            if let Some(index) = gslot.checked_sub(1) {
                self.gravity_grid.fire(index, reg, &gravity_presets);
            }
        }
        self.gravity_grid
            .tick(dt, self.modulation.clock.beats, reg, &gravity_presets);
    }

    /// Recall a preset when `/preset/recall` has moved to a new index.
    ///
    /// Edge-triggered, not level-triggered. A MIDI button or an OSC client
    /// parked on an index would otherwise re-apply that preset every
    /// frame, which pins every parameter it names and makes them
    /// impossible to adjust by hand — the control would feel broken rather
    /// than latched.
    ///
    /// Slot 0 means "nothing selected", and presets start at 1. That is
    /// what makes startup safe *by construction*: the control rests at 0,
    /// so the first frame has nothing to recall and cannot stamp preset 0
    /// over the defaults. It also keeps the first preset reachable —
    /// number it from 0 and it can never be fired from a fresh start,
    /// because the control is already sitting on it.
    ///
    /// A slot past the end of the list is ignored, and still recorded, so
    /// a fader swept across the full range settles quietly instead of
    /// retrying every frame.
    fn apply_pending_preset(&mut self) {
        let reg = &self.params.registry;
        let slot = reg.target(self.params.preset_recall).round().max(0.0) as usize;
        if self.last_preset == Some(slot) {
            return;
        }
        self.last_preset = Some(slot);
        let Some(index) = slot.checked_sub(1) else {
            return;
        };
        match vizz_mod::preset::by_index(index) {
            Some((name, preset)) => {
                let applied = preset.apply(reg);
                // A look transition in flight would rewrite these same
                // parameters on its very next frame, silently eating the
                // recall — the number key would appear to do nothing.
                // The recall is edge-triggered, so reaching here means it
                // is the thing most recently touched; it wins.
                self.grid.halt();
                log::info!("recalled preset {slot}: {name} ({applied} parameters)");
            }
            None => log::debug!("no preset in slot {slot}"),
        }
    }

    /// Forget the last recall edge, so the next tick re-applies whatever
    /// slot `/preset/recall` is sitting on. The number keys call this on
    /// every press: recall is edge-triggered (see `apply_pending_preset`),
    /// so without it pressing the key for the preset already showing does
    /// nothing — and "press the number again to snap back after tweaking"
    /// is exactly what the keys are for.
    pub fn retrigger_preset(&mut self) {
        self.last_preset = None;
    }

    /// The `/preset/recall` slot last acted on, 1-based; `None` when
    /// nothing has been recalled. For the preset row, which had no way to
    /// show *where you are* — every button looked identical whether its
    /// look was on screen or not.
    pub fn current_preset(&self) -> Option<usize> {
        self.last_preset.filter(|s| *s > 0)
    }

    /// Advance time and parameters; returns everything the scene needs.
    /// `fixed_dt` pins the timestep (headless benchmarking); `None` uses
    /// wall-clock time (live).
    pub fn begin_frame(&mut self, aspect: f32, fixed_dt: Option<Duration>) -> FrameInputs {
        let now = Instant::now();
        let dt = fixed_dt.unwrap_or_else(|| {
            let dt = self
                .last_frame
                .map_or(Duration::from_nanos(16_666_667), |t| now - t);
            // Clamp: a debugger pause or window drag must not fast-forward
            // the visuals or the parameter smoothing.
            dt.min(Duration::from_millis(100))
        });
        self.last_frame = Some(now);

        let dt_s = dt.as_secs_f32();
        // Recall before taking a borrow of `params`, and before smoothing
        // advances, so a preset's values are targets for this same frame
        // and the glide starts immediately rather than one frame late.
        self.apply_pending_preset();
        // The page before anything played on it: a deck arriving over MIDI
        // or OSC on the same frame as a pad press has to be the deck that
        // pad belongs to.
        self.decks_dirty |= self.tick_decks();
        // Then the grid, so a scene fired on this frame starts blending
        // on it. A recall on this frame has already halted any transition
        // in flight — both are edge-triggered, so whichever the user
        // touched last is the one writing the parameters.
        self.tick_grid(dt_s);
        let p = Arc::clone(&self.params);
        let p = &*p;
        for (i, b) in self.bands.iter_mut().enumerate() {
            *b = self.audio.state.band(i);
        }
        // Detected tempo drives the clock only when asked and only when the
        // detector is sure. Ambient material still produces *a* peak, and
        // letting that retune the clock mid-set is worse than a tempo that
        // is slightly stale.
        // try_lock: the analysis thread takes this ~94 times a second,
        // and a render thread parked behind a preempted holder is a
        // missed vsync for nothing — a one-frame-stale reading serves
        // auto-bpm exactly as well.
        if let Ok(settings) = self.audio.settings.try_lock()
            && settings.auto_bpm {
                let bpm = self.audio.state.bpm();
                if bpm > 0.0 && self.audio.state.confidence() >= settings.min_confidence {
                    self.modulation.clock.bpm = bpm;
                }
            }
        // Modulation is an offset on top of the stored targets, so a value
        // set by hand or by MIDI is never overwritten.
        let levels = AudioLevels {
            bands: &self.bands,
            level: self.audio.state.level(),
        };
        let offsets = self.modulation.tick(dt_s, &p.registry, levels);
        self.snapshot.advance_modulated(&p.registry, dt_s, offsets);
        // After the slew, not instead of it: modulation still rides on
        // top, and a cut lands the *set* value rather than freezing
        // whatever an LFO happened to be adding.
        if std::mem::take(&mut self.cut_pending) {
            self.snapshot.snap(&p.registry);
        }
        // Freeze holds the picture: visual time stops advancing, and the
        // feedback pass is pinned to full trail below so the last frame
        // survives unchanged. Parameters keep moving underneath — a
        // transition in flight lands while frozen and shows on release,
        // which is the gesture's contract: hold the picture, not the set.
        let frozen = self.snapshot.get(p.punch_freeze) >= 0.5;
        if !frozen {
            self.vis_time += (dt_s * self.snapshot.get(p.speed)) as f64;
        }
        // The strobe's dark phase rides the same darkening as the black
        // punch, so blackout and strobe cost one uniform between them.
        // Computed from the beat clock after modulation ticked it, so the
        // flashes land on the divisions the clock is actually on.
        // A strobe with the clock stopped does nothing rather than
        // parking wherever the beat froze — stuck in the dark phase it
        // would read as a blackout that no button explains.
        let strobe = self.snapshot.get(p.punch_strobe);
        let strobe_dark = if strobe > 0.001 && self.modulation.clock.running {
            let div = self.snapshot.get(p.punch_strobe_div).max(0.05) as f64;
            let lit = (self.modulation.clock.beats / div).fract() < 0.3;
            if lit { 0.0 } else { strobe }
        } else {
            0.0
        };

        // Master dim multiplies *everything* that emits light. It is the
        // fader you grab when something is wrong on a big screen, so
        // anything it does not reach is a thing still lit when you have
        // asked for black — the room used to be exactly that.
        let dim = self.snapshot.get(p.dim);
        let brightness = self.snapshot.get(p.brightness) * dim;

        let camera = Camera {
            distance: self.snapshot.get(p.cam_dist),
            orbit: self.snapshot.get(p.cam_orbit),
            elevation: self.snapshot.get(p.cam_elev),
            fov: self.snapshot.get(p.cam_fov),
            aspect,
            focus: self.snapshot.get(p.cam_focus),
            defocus: self.snapshot.get(p.cam_defocus),
            pan_x: self.snapshot.get(p.cam_pan_x),
            pan_y: self.snapshot.get(p.cam_pan_y),
        };
        let cam = camera.uniforms();
        let room_brightness = self.snapshot.get(p.room) * dim;
        // The opening sits a little in front of the origin so the cloud is
        // inside the room rather than pressed against its face.
        let room = RoomUniforms::for_camera(
            &camera,
            (camera.distance - 1.6).max(0.3),
            self.snapshot.get(p.room_depth),
            room_brightness,
            self.snapshot.get(p.room_fade),
            self.snapshot.get(p.room_converge),
            self.snapshot.get(p.room_vanish_x),
            self.snapshot.get(p.room_vanish_y),
        );
        // Placement is derived even when the room is invisible: embedding
        // the cloud in a room you have not turned up is a legitimate look,
        // and it costs a struct copy.
        let placement = room.placement(
            self.snapshot.get(p.room_anchor),
            self.snapshot.get(p.room_embed),
        );

        // The vector stack. Paper shares the /bg colour the clear uses,
        // sRGB-encoded as registered, and rides the master dim inside
        // the shader (globals lane) rather than pre-multiplied — the
        // shader dims the composited page, which is what a printed tint
        // under a fader should do. Alpha is not carried: the vector
        // page is opaque by construction, and the transparent-master
        // routing feature applies only while the stack is off.
        let mut vector = vizz_render::vector::StackU {
            globals: [aspect, self.vis_time as f32, dim, p.vector_layers.len() as f32],
            bg: [
                self.snapshot.get(p.bg_r),
                self.snapshot.get(p.bg_g),
                self.snapshot.get(p.bg_b),
                0.0, // render height, filled by the caller
            ],
            ..Default::default()
        };
        for (slot, ids) in p.vector_palette.iter().enumerate() {
            vector.palette[slot] = [
                self.snapshot.get(ids[0]),
                self.snapshot.get(ids[1]),
                self.snapshot.get(ids[2]),
                1.0,
            ];
        }
        let mut vector_active = false;
        for (i, l) in p.vector_layers.iter().enumerate() {
            let kind = self.snapshot.get(l.kind).round();
            vector_active |= kind >= 0.5;
            vector.layers[i] = vizz_render::vector::LayerU {
                xform: [
                    self.snapshot.get(l.x),
                    self.snapshot.get(l.y),
                    self.snapshot.get(l.rot),
                    self.snapshot.get(l.scale),
                ],
                // Phase advances with visual time so the whole stack
                // drifts at /particles/speed's rate like everything
                // else; the parameter is the offset on top, and the
                // rate is `drift` rather than a constant hidden here.
                pat: [
                    kind,
                    self.snapshot.get(l.freq),
                    self.snapshot.get(l.phase)
                        + self.vis_time as f32 * self.snapshot.get(l.drift),
                    self.snapshot.get(l.duty),
                ],
                shape: [
                    self.snapshot.get(l.sides),
                    self.snapshot.get(l.inset),
                    self.snapshot.get(l.fold),
                    self.snapshot.get(l.invert).round(),
                ],
                style: [
                    self.snapshot.get(l.blend).round(),
                    self.snapshot.get(l.opacity),
                    self.snapshot.get(l.color).round(),
                    0.0,
                ],
            };
        }

        FrameInputs {
            uniforms: Uniforms {
                view_proj: cam.view_proj,
                cam_right: cam.right,
                focus: camera.focus,
                cam_up: cam.up,
                defocus: camera.defocus,
                cam_position: cam.position,
                _pad_cam: 0.0,
                time: self.vis_time as f32,
                aspect,
                size: self.snapshot.get(p.size),
                spread: self.snapshot.get(p.spread),
                hue: self.snapshot.get(p.hue),
                saturation: self.snapshot.get(p.saturation),
                brightness,
                shape: self.snapshot.get(p.shape),
                morph: self.snapshot.get(p.morph),
                twist: self.snapshot.get(p.twist),
                palette: self.snapshot.get(p.palette),
                color_spread: self.snapshot.get(p.color_spread),
                // Stepped like mirror: a value sliding between two drive
                // modes is not a crossfade, it is a wrong third thing.
                color_drive: self.snapshot.get(p.color_drive).round(),
                gravity: std::array::from_fn(|i| match p.gravity.get(i) {
                    Some(w) => [
                        self.snapshot.get(w.x),
                        self.snapshot.get(w.y),
                        self.snapshot.get(w.z),
                        self.snapshot.get(w.strength),
                    ],
                    None => [0.0; 4],
                }),
                gravity_radius: std::array::from_fn(|i| {
                    p.gravity.get(i).map_or(1.0, |w| self.snapshot.get(w.radius))
                }),
                // The master dim does not scale gravity: it is a shape
                // control, and fading the output to black should not also
                // straighten the cloud out on the way down.
                gravity_amount: [self.snapshot.get(p.gravity_amount), 0.0, 0.0, 0.0],
                // Filled by the caller, which owns the palette bank.
                palette_rows: [4.0, 0.0, 0.0, 0.0],
                // Presence and aspect are the renderer's to know — it
                // holds the texture — so they are filled in by the
                // caller alongside the palette count. Only the two
                // controls come from the parameter table here.
                video: [
                    0.0,
                    1.0,
                    self.snapshot.get(p.video_depth),
                    self.snapshot.get(p.video_relief).round(),
                ],
                // Slot choice is stepped; the morph between them is not.
                cloud_a: self.snapshot.get(p.cloud_a).round(),
                cloud_b: self.snapshot.get(p.cloud_b).round(),
                cloud_morph: self.snapshot.get(p.cloud_morph),
                room: placement,
            },
            post: PostUniforms {
                // At trail 1.0 the feedback lerp passes history through
                // unchanged — a genuine frame hold. Zoom and spin still
                // apply, which is a look (a frozen frame you can tunnel).
                trail: if frozen { 1.0 } else { self.snapshot.get(p.trail) },
                zoom: self.snapshot.get(p.zoom),
                spin: self.snapshot.get(p.spin),
                // Rounded: mirror modes are discrete, and a smoothed value
                // sliding between them would flicker between folds.
                mirror: self.snapshot.get(p.mirror).round(),
                glow: self.snapshot.get(p.glow),
                aspect,
                shift: self.snapshot.get(p.shift),
                flash: self.snapshot.get(p.punch_flash),
                invert: self.snapshot.get(p.punch_invert),
                black: self.snapshot.get(p.punch_black).max(strobe_dark),
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            },
            room,
            room_visible: room_brightness > 0.002,
            vector,
            vector_active,
            vector_print: self.snapshot.get(p.vec_place).round() >= 0.5,
            count: self.snapshot.get(p.count).max(0.0) as u32,
            background: wgpu::Color {
                // The master dim scales the background as well as the
                // field. Pulling the master and being left with a lit
                // backdrop would make the one emergency control not work.
                r: (self.snapshot.get(p.bg_r) * dim) as f64,
                g: (self.snapshot.get(p.bg_g) * dim) as f64,
                b: (self.snapshot.get(p.bg_b) * dim) as f64,
                // Alpha is *not* dimmed: transparency is a routing
                // decision, not a brightness one, and fading the master
                // should not quietly make the output opaque.
                a: self.snapshot.get(p.bg_a) as f64,
            },
        }
    }

    /// Record the finished frame; returns a snapshot when the periodic
    /// health log is due (every 2s) so callers can print/display it.
    /// Record how long the frame spent in the UI, before `end_frame`.
    ///
    /// Separate from `end_frame` because headless has no UI at all and
    /// must not report a zero that reads as "the UI is free" — it
    /// reports nothing, and the health line omits the field.
    pub fn end_ui(&mut self, ui_time: Duration) {
        self.health.on_ui(ui_time);
    }

    pub fn end_frame(&mut self, frame_time: Duration) -> Option<HealthSnapshot> {
        self.health.on_frame(frame_time);
        if self.last_log.elapsed() >= Duration::from_secs(2) {
            self.last_log = Instant::now();
            Some(self.health.snapshot())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pattern's drift is a parameter, and zero really is still.
    ///
    /// Reported from use: picking the rings generator produced a slowly
    /// moving picture with nothing to point at — no LFO, no modulation
    /// route, no control. The movement was a constant `0.1` added to the
    /// phase *inside this function*, after the parameter had been read,
    /// so nothing in the app could show it and nothing could stop it.
    ///
    /// A picture that moves with no visible cause is the worst shape a
    /// behaviour can have in an instrument. This asserts the two halves
    /// that fix it: the rate is read from a parameter, and setting that
    /// parameter to zero holds the pattern still.
    ///
    /// Computed the way the frame computes it rather than through a GPU
    /// frame, which needs a device — the arithmetic is the whole claim.
    #[test]
    fn a_layer_holds_still_when_its_drift_is_zero() {
        let params = crate::params::AppParams::build();
        let reg = &params.registry;
        let layer = params.vector_layers[0];
        let phase_at = |vis_time: f32| reg.target(layer.phase) + vis_time * reg.target(layer.drift);

        // Shipped default: the look everyone already has keeps moving.
        assert!(
            (reg.target(layer.drift) - 0.1).abs() < 1e-6,
            "the default drift changed, which changes every saved look"
        );
        assert_ne!(
            phase_at(0.0),
            phase_at(4.0),
            "the default no longer moves at all"
        );

        // Turned off, it is genuinely still — not merely slower.
        reg.set(layer.drift, 0.0);
        assert_eq!(
            phase_at(0.0),
            phase_at(10_000.0),
            "a layer with no drift still walked"
        );

        // And it runs backwards, which a hardcoded constant could not.
        reg.set(layer.drift, -0.5);
        assert!(phase_at(4.0) < phase_at(0.0), "negative drift did not reverse");
    }

    /// Visual time is a running sum of a small increment, which is the
    /// shape that goes wrong quietly. In `f32` the additions start being
    /// swallowed by the accumulator's own rounding as it grows, and an
    /// installation left running overnight is inside the range where it
    /// matters — but nothing shows it in the first minute of a test.
    ///
    /// So this integrates the arithmetic directly, at a magnitude it takes
    /// hours to reach, and asserts the result is the sum rather than
    /// something that stopped moving.
    #[test]
    fn visual_time_still_advances_after_days_of_running() {
        // Six days at 60 Hz. Chosen because that is where an f32
        // accumulator stops advancing at all.
        const HZ: f64 = 60.0;
        const HOURS: f64 = 146.0;
        let step = 1.0 / HZ;

        let mut f64_time: f64 = 0.0;
        let mut f32_time: f32 = 0.0;
        let frames = (HOURS * 3600.0 * HZ) as u64;
        for _ in 0..frames {
            f64_time += step;
            f32_time += step as f32;
        }
        let want = frames as f64 * step;

        // The accumulator this code uses lands on the answer.
        assert!(
            (f64_time - want).abs() < 1.0,
            "f64 drifted: {f64_time} vs {want}"
        );

        // The one it replaced has stopped dead. Not merely inaccurate —
        // frozen, pinned at a power of two, taking additions that change
        // nothing. Another second of frames moves one and not the other,
        // which is the whole failure in one assertion: on screen it is
        // animation that simply stops while the app carries on running.
        let (f64_before, f32_before) = (f64_time, f32_time);
        for _ in 0..(HZ as u64) {
            f64_time += step;
            f32_time += step as f32;
        }
        assert!(f64_time > f64_before, "f64 stopped advancing");
        assert_eq!(
            f32_time, f32_before,
            "f32 was expected to be frozen by now; it still moved"
        );
    }

    fn engine() -> FrameEngine {
        // "\0none" matches no device, so the engine takes its normal
        // unavailable path and reports zeros. No GPU is involved here.
        FrameEngine::new(
            Arc::new(AppParams::build()),
            vizz_audio::AudioEngine::start(Some("\0none")),
        )
    }

    /// Starting up must not recall anything. `/preset/recall` defaults to
    /// 0, and treating the first frame as a change fires preset 0 over the
    /// defaults before the window is even on screen — which is how this
    /// was found: every headless render logged "recalled preset 0".
    #[test]
    fn startup_does_not_recall_a_preset() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let glow = reg.id("/fx/glow").unwrap();
        let before = reg.target(glow);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            reg.target(glow),
            before,
            "startup recalled a preset over the defaults"
        );
    }

    /// Moving the control recalls; staying put does not. Level-triggering
    /// would re-apply every frame and pin every parameter the preset
    /// names, so adjusting one by hand afterwards would be impossible.
    #[test]
    fn recall_fires_on_change_and_only_on_change() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        // Slot 1 is the first built-in, so this must change something.
        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        let recalled = reg.target(glow);
        let expected = vizz_mod::preset::BUILTINS[0]
            .preset()
            .values
            .get("/fx/glow")
            .copied()
            .expect("the first built-in should set glow");
        assert!((recalled - expected).abs() < 1e-6, "preset did not apply");

        // Now move it by hand with recall parked. A second frame must not
        // stamp the preset back over the top.
        reg.set_by_addr("/fx/glow", 0.02);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(
            (reg.target(glow) - 0.02).abs() < 1e-6,
            "a parked recall re-applied its preset and fought the user"
        );
    }

    /// Freeze holds the picture: visual time must stop dead the frame
    /// the gesture lands and resume from the same phase on release —
    /// while the feedback pass is pinned to full trail so the last frame
    /// survives.
    #[test]
    fn freeze_stops_visual_time_and_pins_the_trail() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        e.begin_frame(16.0 / 9.0, dt);

        reg.set_by_addr("/punch/freeze", 1.0);
        let a = e.begin_frame(16.0 / 9.0, dt);
        let b = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(a.uniforms.time, b.uniforms.time, "time advanced while frozen");
        assert_eq!(b.post.trail, 1.0, "trail not pinned while frozen");

        reg.set_by_addr("/punch/freeze", 0.0);
        let c = e.begin_frame(16.0 / 9.0, dt);
        assert!(c.uniforms.time > b.uniforms.time, "time did not resume");
        assert!(c.post.trail < 1.0, "trail still pinned after release");
    }

    /// A flash that fades in is not a flash: the full value must reach
    /// the GPU on the very frame it was set. This is the test that fails
    /// if someone gives the punch params a smoothing constant.
    #[test]
    fn a_flash_lands_at_full_strength_on_the_same_frame() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        e.begin_frame(16.0 / 9.0, dt);

        reg.set_by_addr("/punch/flash", 1.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.flash, 1.0, "flash was smoothed on its way to the GPU");

        reg.set_by_addr("/punch/flash", 0.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.flash, 0.0, "flash lingered after release");
    }

    /// The strobe alternates lit and dark phases on the beat clock, and
    /// the dark phase rides the same uniform as the blackout.
    #[test]
    fn the_strobe_alternates_on_the_beat() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        // 60 bpm = one beat per second; a quarter-beat division makes a
        // full strobe cycle every 250 ms, sampled well by 16 ms frames.
        e.modulation.clock.bpm = 60.0;
        let dt = Some(Duration::from_millis(16));
        reg.set_by_addr("/punch/strobe", 1.0);
        reg.set_by_addr("/punch/strobe_div", 0.25);

        let (mut lit, mut dark) = (0, 0);
        for _ in 0..60 {
            let f = e.begin_frame(16.0 / 9.0, dt);
            if f.post.black > 0.5 {
                dark += 1;
            } else {
                lit += 1;
            }
        }
        assert!(lit > 5, "strobe never lit ({lit} lit / {dark} dark)");
        assert!(dark > 5, "strobe never went dark ({lit} lit / {dark} dark)");

        reg.set_by_addr("/punch/strobe", 0.0);
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert_eq!(f.post.black, 0.0, "strobe left the black uniform up");
    }

    /// Recalling a look must never replay somebody's blackout: no punch
    /// gesture may be captured into a preset.
    #[test]
    fn a_captured_preset_carries_no_punch_state() {
        let e = engine();
        let reg = &e.params.registry;
        reg.set_by_addr("/punch/black", 1.0);
        reg.set_by_addr("/punch/freeze", 1.0);
        let look = vizz_mod::preset::Preset::capture(reg);
        assert!(
            !look.values.keys().any(|a| a.starts_with("/punch/")),
            "a preset captured punch state: {:?}",
            look.values.keys().filter(|a| a.starts_with("/punch/")).collect::<Vec<_>>()
        );
    }

    /// A recall must survive a look transition in flight. The grid
    /// writes its blend into the same parameters every frame, so without
    /// halting it the recalled preset was on screen for one frame and
    /// then silently overwritten — the number key reads as doing nothing.
    #[test]
    fn a_recall_wins_over_a_transition_in_flight() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        // Pad 1 plays the second built-in; fire it and make sure the
        // blend is genuinely in flight before recalling over it.
        e.grid.assign(0, vizz_mod::preset::BUILTINS[1].name);
        reg.set_by_addr("/scene/fire", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(e.grid.in_flight().is_some(), "no transition to survive");

        // Recall the first built-in mid-blend, then give the transition
        // more than enough frames to have stamped its target if it were
        // still alive.
        reg.set_by_addr("/preset/recall", 1.0);
        for _ in 0..240 {
            e.begin_frame(16.0 / 9.0, dt);
        }
        let expected = vizz_mod::preset::BUILTINS[0].preset().values["/fx/glow"];
        assert!(
            (reg.target(glow) - expected).abs() < 1e-6,
            "the in-flight transition overwrote the recall: glow {} vs {expected}",
            reg.target(glow)
        );
    }

    /// Pressing the number key for the preset already showing must
    /// re-apply it. Recall is edge-triggered and the key writes the same
    /// slot value, so without `retrigger_preset` the press moves nothing
    /// — and snapping back after hand-tweaking is the main thing the
    /// number keys are pressed for.
    #[test]
    fn a_repeated_number_key_reapplies_the_preset() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();

        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        let recalled = reg.target(glow);

        // Tweak by hand; the parked recall must not fight it...
        reg.set_by_addr("/fx/glow", 0.02);
        e.begin_frame(16.0 / 9.0, dt);
        assert!((reg.target(glow) - 0.02).abs() < 1e-6);

        // ...but the same key pressed again snaps it back.
        e.retrigger_preset();
        reg.set_by_addr("/preset/recall", 1.0);
        e.begin_frame(16.0 / 9.0, dt);
        assert!(
            (reg.target(glow) - recalled).abs() < 1e-6,
            "the repeated press did nothing"
        );
    }

    /// The master dim is the fader you reach for when something is wrong
    /// in front of an audience. Anything it fails to reach is still lit
    /// when you have asked for black — the room was, until this test.
    #[test]
    fn the_master_dim_blacks_out_the_room_too() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        reg.set_by_addr("/room/brightness", 1.0);
        // Dim is smoothed, so run long enough for it to arrive.
        reg.set_by_addr("/master/dim", 0.0);
        for _ in 0..120 {
            e.begin_frame(16.0 / 9.0, dt);
        }
        let f = e.begin_frame(16.0 / 9.0, dt);
        assert!(
            f.room.brightness < 0.01,
            "room still lit at {} with the master dim down",
            f.room.brightness
        );
        assert!(
            !f.room_visible,
            "room pass still running with the master dim down"
        );
    }

    /// An index past the end is ignored rather than applying whatever is
    /// nearest, and sweeping a fader through empty indices must not spam
    /// or reset anything.
    #[test]
    fn an_empty_index_changes_nothing() {
        let mut e = engine();
        let reg = Arc::clone(&e.params.registry);
        let dt = Some(Duration::from_millis(16));
        let glow = reg.id("/fx/glow").unwrap();
        reg.set_by_addr("/fx/glow", 0.33);
        for index in [40.0, 41.0, 63.0] {
            reg.set_by_addr("/preset/recall", index);
            e.begin_frame(16.0 / 9.0, dt);
            assert!(
                (reg.target(glow) - 0.33).abs() < 1e-6,
                "index {index} disturbed a parameter"
            );
        }
    }

    // ---- decks ----------------------------------------------------------

    /// A book of two pages with one pad filled on each, so a page turn
    /// has something to be right or wrong about.
    fn two_decks(e: &mut FrameEngine) {
        e.grid.assign(0, "Slow bloom");
        e.decks.store(&e.grid, &e.gravity_grid);
        e.decks
            .add(&mut e.grid, &mut e.gravity_grid)
            .expect("a second deck");
        e.grid.assign(1, "Tunnel");
        e.after_deck_change();
        e.decks.switch(0, &mut e.grid, &mut e.gravity_grid);
        e.after_deck_change();
    }

    /// Starting up must not turn a page, for the same reason it must not
    /// recall a preset: `/deck/select` rests at 0, and treating the first
    /// frame as a change would store the loaded grids into deck 1 and load
    /// deck 1 back over them before the window is on screen.
    #[test]
    fn startup_does_not_turn_a_page() {
        let mut e = engine();
        two_decks(&mut e);
        assert_eq!(e.decks.active(), 0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 0, "startup turned a page");
        assert_eq!(
            e.grid.cell(0).map(|c| c.preset.as_str()),
            Some("Slow bloom"),
            "startup disturbed the pads"
        );
    }

    /// The gesture: writing the parameter turns the page, and the pads
    /// underneath change with it.
    #[test]
    fn selecting_a_deck_swaps_the_pads() {
        let mut e = engine();
        two_decks(&mut e);
        let reg = Arc::clone(&e.params.registry);

        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1);
        assert_eq!(e.grid.cell(1).map(|c| c.preset.as_str()), Some("Tunnel"));
        assert_eq!(e.grid.cell(0), None, "the first deck's pads came along");

        reg.set(e.params.deck_select, 1.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 0);
        assert_eq!(e.grid.cell(0).map(|c| c.preset.as_str()), Some("Slow bloom"));
    }

    /// Turning a page must not play anything. Every page turn happens in
    /// front of an audience, and a scene firing itself because the
    /// parameter still held the last pad's number would be a picture
    /// change nobody asked for.
    ///
    /// This is the failure the re-arm in `after_deck_change` exists for:
    /// clearing the latch alone left `/scene/fire` holding slot 1, and the
    /// very next frame read that as a change.
    #[test]
    fn turning_a_page_does_not_fire_a_scene() {
        let mut e = engine();
        two_decks(&mut e);
        let reg = Arc::clone(&e.params.registry);

        // Play the first deck's pad, and let the cut land. The blend
        // time goes through the parameter because `tick_grid` reads it
        // back over the field every frame.
        reg.set(e.params.scene_time, 0.0);
        reg.set(e.params.scene_fire, 1.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.grid.current(), Some(0), "the scene never fired");

        // Deck 2 has its pad at slot 2, so slot 1 is empty there. If the
        // page turn re-fired, `current` would move; if it left the latch
        // alone, the pad could never be pressed again.
        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1);
        assert_eq!(
            reg.target(e.params.scene_fire),
            0.0,
            "the fire control did not go back to rest, so the next frame fires the old slot"
        );

        // And the same pad number is live again straight away, which is
        // the other half of the trade.
        reg.set(e.params.scene_fire, 1.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            e.grid.in_flight().map(|(s, _)| s).or(e.grid.current()),
            None,
            "slot 1 is empty on this deck, so nothing should have fired"
        );
        reg.set(e.params.scene_fire, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.grid.current(), Some(1), "the new deck's pad would not fire");
    }

    /// A chip stays live after a page is created, copied or deleted.
    ///
    /// The parameter is the address of the live page and is edge-triggered
    /// on its value, but `+`, duplicate and delete reach the book directly
    /// — so the parameter went on naming the chip that was live before the
    /// gesture. Clicking that chip then wrote the number it already held:
    /// no edge, no turn, and the chip was dead until some other chip was
    /// clicked first. Found by review, reproduced here before the fix.
    #[test]
    fn a_chip_is_not_dead_after_a_page_is_added() {
        let mut e = engine();
        two_decks(&mut e);
        let reg = Arc::clone(&e.params.registry);

        // On deck 2 by its chip.
        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1);

        // "+" — the book moves without the parameter hearing about it.
        e.decks.add(&mut e.grid, &mut e.gravity_grid).expect("a third deck");
        e.after_deck_change();
        assert_eq!(e.decks.active(), 2);

        // Deck 2's chip again. This is the click that did nothing.
        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            e.decks.active(),
            1,
            "the chip for the page selected before the '+' is dead"
        );
        assert_eq!(
            e.grid.cell(1).map(|c| c.preset.as_str()),
            Some("Tunnel"),
            "the page turned but its pads did not come with it"
        );
    }

    /// And the same after a delete, which renumbers the pages under the
    /// parameter rather than merely moving past them.
    #[test]
    fn a_chip_is_not_dead_after_a_page_is_deleted() {
        let mut e = engine();
        two_decks(&mut e);
        let reg = Arc::clone(&e.params.registry);
        e.decks.add(&mut e.grid, &mut e.gravity_grid).expect("a third deck");
        e.after_deck_change();

        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1);

        // Delete the page below the live one: everything shifts down.
        assert!(e.decks.remove(0, &mut e.grid, &mut e.gravity_grid));
        e.after_deck_change();
        assert_eq!(e.decks.active(), 0);

        // The chip now at position 2 is the one that was 3.
        reg.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1, "the chip below the deleted page is dead");
    }

    /// Reseating the page control must not itself be read as a selection
    /// on the next frame, or a "+" would turn straight back off the page
    /// it just made.
    ///
    /// This is the half-fix: clearing the latch and leaving the parameter
    /// naming the page you were on. The next frame reads that as a change
    /// and turns back, so the "+" appears to do nothing at all. The
    /// parameter has to move with the latch, which is why the fix is two
    /// lines and not one.
    ///
    /// The deck must be selected *through the parameter* first, or it
    /// rests at 0 and there is no stale value to turn back to — which is
    /// how the first version of this test passed against the half-fix.
    #[test]
    fn adding_a_page_stays_on_the_page_it_added() {
        let mut e = engine();
        two_decks(&mut e);
        e.params.registry.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 1, "the fixture never reached deck 2");

        e.decks.add(&mut e.grid, &mut e.gravity_grid).expect("a third deck");
        e.after_deck_change();
        for _ in 0..3 {
            e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        }
        assert_eq!(e.decks.active(), 2, "the new page was turned away from");
    }

    /// A page that does not exist is silence, and the edge is still
    /// recorded — a fader swept across a bank of eight buttons on a show
    /// with two decks must not retry the missing ones every frame.
    #[test]
    fn selecting_a_page_that_does_not_exist_does_nothing() {
        let mut e = engine();
        two_decks(&mut e);
        let reg = Arc::clone(&e.params.registry);
        reg.set(e.params.deck_select, 9.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.decks.active(), 0, "a page past the end was selected anyway");
        assert_eq!(e.grid.cell(0).map(|c| c.preset.as_str()), Some("Slow bloom"));
    }

    /// A column is the scene pad and the gravity pad of the same number,
    /// together. That is what a column means in the program this follows,
    /// and it is why one address drives two grids.
    #[test]
    fn a_column_fires_both_grids() {
        let mut e = engine();
        e.grid.assign(2, "Slow bloom");
        e.gravity_grid.assign(2, "a well");
        let reg = Arc::clone(&e.params.registry);

        reg.set(e.params.column_fire, 3.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(reg.target(e.params.scene_fire), 3.0);
        assert_eq!(reg.target(e.params.gravity_fire), 3.0);
    }

    /// Relaunching the column already showing has to land. Firing is edge
    /// triggered on the slot number, which is right for a pad — pressing 5
    /// twice is one move — and wrong for a column, where a relaunch is a
    /// deliberate re-trigger. The listener's counter is what carries it.
    #[test]
    fn relaunching_a_column_fires_it_again() {
        let mut e = engine();
        e.grid.assign(0, "Slow bloom");
        e.grid.assign(1, "Tunnel");
        let reg = Arc::clone(&e.params.registry);
        reg.set(e.params.scene_time, 0.0);

        reg.set(e.params.column_fire, 1.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.grid.current(), Some(0));

        // Move away by hand, then relaunch the same column. The value has
        // not changed, so only the counter can say this happened.
        reg.set(e.params.scene_fire, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.grid.current(), Some(1));

        e.columns.fires.fetch_add(1, std::sync::atomic::Ordering::Release);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            e.grid.current(),
            Some(0),
            "relaunching the same column left the grid where it was"
        );
    }

    /// A column that arrived before the window opened is caught up to
    /// exactly once.
    ///
    /// The listener binds before the engine exists, so by the time it is
    /// adopted `/column/fire` can already hold the column Arena is
    /// showing. Landing on it once is the right answer — it is what makes
    /// vizz agree with Resolume from the first frame rather than from the
    /// next launch. Landing on it *repeatedly* would pin the grid there
    /// and make every pad on the desk dead, which is what the value latch
    /// prevents.
    #[test]
    fn a_column_that_arrived_before_the_window_is_caught_up_once() {
        let mut e = engine();
        e.grid.assign(0, "Slow bloom");
        e.grid.assign(1, "Tunnel");
        let reg = Arc::clone(&e.params.registry);
        reg.set(e.params.scene_time, 0.0);

        let columns = Arc::new(vizz_osc::ColumnSync::default());
        columns.fires.store(7, std::sync::atomic::Ordering::Release);
        reg.set(e.params.column_fire, 1.0);
        e.adopt_column_sync(columns);

        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(e.grid.current(), Some(0), "vizz did not catch up to the live column");

        // Move away by hand. A column that is not relaunched must not drag
        // the grid back on the next frame.
        reg.set(e.params.scene_fire, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            e.grid.current(),
            Some(1),
            "the startup column kept re-firing and pinned the grid"
        );
    }

    /// The live page's stretch of Resolume's composition is mirrored to
    /// the listener, which is the only place that value is read.
    #[test]
    fn the_listeners_column_origin_follows_the_live_deck() {
        use std::sync::atomic::Ordering;
        let mut e = engine();
        two_decks(&mut e);
        e.decks.set_origin(1, 17);
        let columns = Arc::new(vizz_osc::ColumnSync::default());
        e.adopt_column_sync(Arc::clone(&columns));
        assert_eq!(columns.origin.load(Ordering::Relaxed), 1);

        e.params.registry.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert_eq!(
            columns.origin.load(Ordering::Relaxed),
            17,
            "the listener is still following the page that was left"
        );
    }

    /// A page turn is worth writing to disk, and being asked clears the
    /// flag — one save answers one change.
    #[test]
    fn a_page_turn_asks_to_be_saved_once() {
        let mut e = engine();
        two_decks(&mut e);
        assert!(!e.take_decks_dirty(), "nothing has happened yet");
        e.params.registry.set(e.params.deck_select, 2.0);
        e.begin_frame(16.0 / 9.0, Some(Duration::from_millis(16)));
        assert!(e.take_decks_dirty(), "a page turn went unrecorded");
        assert!(!e.take_decks_dirty(), "the same page turn asked to be saved twice");
    }
}
#[cfg(test)]
mod vector_pack_tests {
    use super::*;

    /// The lane map, held to code. `begin_frame` writes each parameter
    /// into a specific component of a specific vec4; getting one wrong
    /// does not fail — it makes a knob move the wrong thing, which on
    /// stage reads as "the app is haunted". Distinctive values in, exact
    /// lanes out.
    #[test]
    fn vector_packing_puts_each_parameter_in_its_lane() {
        let params = std::sync::Arc::new(crate::params::AppParams::build());
        let p = &*params;
        let l3 = p.vector_layers[2];
        p.registry.set(l3.kind, 5.0);
        p.registry.set(l3.freq, 23.0);
        p.registry.set(l3.blend, 4.0);
        p.registry.set(l3.opacity, 0.75);
        p.registry.set(p.vector_palette[2][1], 0.33);

        let mut engine = FrameEngine::new(
            std::sync::Arc::clone(&params),
            vizz_audio::AudioEngine::start(Some("\0none")),
        );
        // Two long steps so the smoothed params reach their targets.
        engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(5)));
        let inputs = engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(5)));

        let l = &inputs.vector.layers[2];
        assert_eq!(l.pat[0], 5.0, "kind lane");
        assert!((l.pat[1] - 23.0).abs() < 0.05, "freq lane: {}", l.pat[1]);
        assert_eq!(l.style[0], 4.0, "blend lane");
        assert!((l.style[1] - 0.75).abs() < 0.02, "opacity lane: {}", l.style[1]);
        assert!(
            (inputs.vector.palette[2][1] - 0.33).abs() < 0.02,
            "palette lane: {}",
            inputs.vector.palette[2][1]
        );
        assert!(inputs.vector_active, "a layer with a kind is an active stack");

        // And the guarantee the render order depends on: everything at
        // defaults means inactive, so the pass is skipped and the frame
        // is byte-identical to the pre-vector app.
        let fresh = std::sync::Arc::new(crate::params::AppParams::build());
        let mut engine = FrameEngine::new(
            std::sync::Arc::clone(&fresh),
            vizz_audio::AudioEngine::start(Some("\0none")),
        );
        let inputs = engine.begin_frame(16.0 / 9.0, Some(std::time::Duration::from_secs(1)));
        assert!(!inputs.vector_active, "defaults must leave the stack off");
    }

}
