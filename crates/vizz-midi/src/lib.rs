//! MIDI input: hardware controllers driving vizz parameters.
//!
//! Like OSC, MIDI is a control-thread citizen — it writes normalised
//! targets into the shared [`ParamRegistry`] and never touches the
//! renderer. Devices are treated as hot-pluggable: ports are rescanned
//! periodically, so plugging a controller in mid-set connects it, and
//! unplugging one does not disturb anything else.
//!
//! Shared state (mappings, learn target, connected devices) lives behind
//! one mutex. The GUI reads it with `try_lock` and falls back to its last
//! snapshot, so the render thread can never be blocked by MIDI traffic.

pub mod mapping;
pub mod message;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context as _, Result};
use midir::{MidiInput, MidiInputConnection};
use vizz_params::ParamRegistry;

pub use mapping::{Binding, Dispatcher, MidiMap, Source, Update};
pub use message::MidiEvent;

/// How often to rescan for newly plugged-in controllers.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// What a pending learn will bind the next control to.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnTarget {
    /// Parameter address to bind.
    pub param: String,
    /// The fixed value a press should send, for a control that addresses a
    /// slot rather than sweeping a range. See [`Binding::value`].
    pub value: Option<f32>,
    /// What to call this while the learn is waiting. The address is right
    /// for a slider — it is what the row is labelled with — but "waiting
    /// for a control to bind to /scene/fire = 5" is not how anyone thinks
    /// about the pad they just armed.
    pub label: String,
}

impl LearnTarget {
    /// Learn a control that sweeps the parameter's range.
    pub fn param(param: impl Into<String>) -> Self {
        let param = param.into();
        Self { label: param.clone(), param, value: None }
    }

    /// Learn a button that jumps the parameter to one value.
    pub fn value(param: impl Into<String>, value: f32, label: impl Into<String>) -> Self {
        Self { param: param.into(), value: Some(value), label: label.into() }
    }
}

/// State shared between the MIDI thread and the UI.
#[derive(Default)]
pub struct MidiState {
    pub map: MidiMap,
    /// What is awaiting a control, set by the GUI's Learn button.
    pub learn_target: Option<LearnTarget>,
    /// Names of currently connected input ports.
    pub connected: Vec<String>,
    /// Last source seen, for "is it even sending?" feedback while learning.
    pub last_source: Option<Source>,
    /// Bumped whenever a binding changes, so the app knows to save.
    pub revision: u64,
}

impl MidiState {
    /// Bind the learn target if one is pending. Returns true if a binding
    /// was made.
    fn apply_learn(&mut self, source: Source) -> bool {
        let Some(target) = self.learn_target.take() else { return false };
        match target.value {
            Some(v) => self.map.bind_value(source, target.param, v),
            None => self.map.bind(source, target.param),
        }
        self.revision += 1;
        true
    }

    /// Is a sweep learn pending for this parameter? Used to light the
    /// control's own learn button.
    pub fn learning(&self, param: &str) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value.is_none())
    }

    /// Is a trigger learn pending for this value of this parameter?
    pub fn learning_value(&self, param: &str, value: f32) -> bool {
        matches!(&self.learn_target, Some(t) if t.param == param && t.value == Some(value))
    }
}

pub type SharedMidi = Arc<Mutex<MidiState>>;

/// Running MIDI input. Dropping it disconnects and stops the thread.
pub struct MidiEngine {
    shared: SharedMidi,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MidiEngine {
    /// Start listening on every available input port.
    ///
    /// Failing to open MIDI at all is *not* fatal — the visuals and OSC
    /// keep working — so callers should log and continue.
    pub fn spawn(registry: Arc<ParamRegistry>, shared: SharedMidi) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_shared = Arc::clone(&shared);

        let thread = std::thread::Builder::new()
            .name("vizz-midi".into())
            .spawn(move || run(registry, thread_shared, thread_stop))?;

        Ok(Self { shared, stop, thread: Some(thread) })
    }

    pub fn shared(&self) -> &SharedMidi {
        &self.shared
    }
}

impl Drop for MidiEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Owns the connections; rescans so controllers can be plugged in mid-set.
fn run(registry: Arc<ParamRegistry>, shared: SharedMidi, stop: Arc<AtomicBool>) {
    // Connections must be kept alive; dropping one closes the port.
    let mut open: Vec<(String, MidiInputConnection<()>)> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        match scan_and_connect(&registry, &shared, &mut open) {
            Ok(()) => {}
            // A transient enumeration failure must not kill MIDI for the
            // rest of the session; try again on the next pass.
            Err(e) => log::debug!("MIDI rescan failed: {e:#}"),
        }
        std::thread::sleep(RESCAN_INTERVAL);
    }
    log::info!("MIDI input stopped");
}

fn scan_and_connect(
    registry: &Arc<ParamRegistry>,
    shared: &SharedMidi,
    open: &mut Vec<(String, MidiInputConnection<()>)>,
) -> Result<()> {
    let input = MidiInput::new("vizz")?;
    let ports = input.ports();
    let names: Vec<String> = ports
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();

    // Drop connections whose device disappeared.
    open.retain(|(name, _)| {
        let still_there = names.contains(name);
        if !still_there {
            log::info!("MIDI device disconnected: {name}");
        }
        still_there
    });

    for port in &ports {
        let Ok(name) = input.port_name(port) else { continue };
        if open.iter().any(|(n, _)| n == &name) {
            continue;
        }
        // Each connection consumes a MidiInput, so build a fresh one.
        let conn_input = MidiInput::new("vizz")?;
        let registry = Arc::clone(registry);
        let shared = Arc::clone(shared);
        let mut dispatcher = Dispatcher::default();
        let label = name.clone();

        match conn_input.connect(
            port,
            "vizz-in",
            move |_stamp, bytes, _| {
                handle_message(bytes, &registry, &shared, &mut dispatcher);
            },
            (),
        ) {
            Ok(conn) => {
                log::info!("MIDI device connected: {label}");
                open.push((name, conn));
            }
            Err(e) => log::debug!("could not open MIDI port {label}: {e}"),
        }
    }

    if let Ok(mut state) = shared.lock() {
        state.connected = open.iter().map(|(n, _)| n.clone()).collect();
    }
    Ok(())
}

/// Called from midir's callback thread for every incoming message.
fn handle_message(
    bytes: &[u8],
    registry: &ParamRegistry,
    shared: &SharedMidi,
    dispatcher: &mut Dispatcher,
) {
    let Some(event) = message::parse(bytes) else { return };

    // Poisoned mutex would mean a panic elsewhere; dropping the message is
    // better than propagating the panic into the MIDI callback thread.
    let Ok(mut state) = shared.lock() else { return };
    let source = Dispatcher::learn_source(event);
    state.last_source = Some(source);

    if state.apply_learn(source) {
        log::info!("MIDI learned: {} -> {:?}", source.label(), state.map.param_for(&source));
        return;
    }

    let resolved = dispatcher.resolve(event, &state.map);
    drop(state); // release before touching the registry

    if let Some((param, update)) = resolved
        && let Some(id) = registry.id(&param)
    {
        match update {
            Update::Range(t) => registry.set_normalized(id, t),
            // Straight through. `set` clamps to the parameter's range, so
            // a binding left behind by a parameter that has since shrunk
            // lands somewhere valid rather than being dropped.
            Update::Absolute(v) => registry.set(id, v),
        }
    }
}

/// Where mappings live by default.
pub fn default_map_path() -> PathBuf {
    std::env::home_dir()
        .map(|h| h.join(".config/vizz/midi.json"))
        .unwrap_or_else(|| PathBuf::from("vizz-midi.json"))
}

/// Load a mapping file. A missing file is not an error — it just means no
/// mappings yet.
pub fn load_map(path: &Path) -> Result<MidiMap> {
    if !path.exists() {
        return Ok(MidiMap::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

pub fn save_map(path: &Path, map: &MidiMap) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(map)?;
    // Temp file then rename, matching every other persisted artefact in
    // the workspace. A plain write truncates first, so a crash, a power
    // loss or a full disk during it leaves an empty or half-written file,
    // `load_map` fails, and the app starts with no mappings — losing the
    // most laborious setup in the app to the narrowest of windows.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_params::ParamDef;

    fn registry() -> Arc<ParamRegistry> {
        let mut b = ParamRegistry::builder();
        b.add(ParamDef::new("/master/dim", 0.0, 1.0, 1.0));
        b.add(ParamDef::new("/particles/hue", 0.0, 1.0, 0.5));
        // A slot parameter, to stand in for the real `/scene/fire`.
        b.add(ParamDef::new("/scene/fire", 0.0, 16.0, 0.0));
        Arc::new(b.build())
    }

    /// The whole path a real message takes, minus the hardware: parse ->
    /// resolve -> write a normalised value into the registry.
    #[test]
    fn a_bound_cc_moves_the_parameter() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        shared
            .lock()
            .unwrap()
            .map
            .bind(Source::ControlChange { channel: 0, controller: 7 }, "/master/dim");
        let mut d = Dispatcher::default();

        handle_message(&[0xB0, 7, 0], &reg, &shared, &mut d);
        assert_eq!(reg.target(reg.id("/master/dim").unwrap()), 0.0);

        handle_message(&[0xB0, 7, 127], &reg, &shared, &mut d);
        assert_eq!(reg.target(reg.id("/master/dim").unwrap()), 1.0);
    }

    #[test]
    fn learn_binds_the_next_control_and_does_not_also_move_it() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        shared.lock().unwrap().learn_target = Some(LearnTarget::param("/particles/hue"));
        let mut d = Dispatcher::default();

        let before = reg.target(reg.id("/particles/hue").unwrap());
        handle_message(&[0xB2, 20, 100], &reg, &shared, &mut d);

        let state = shared.lock().unwrap();
        assert_eq!(
            state.map.param_for(&Source::ControlChange { channel: 2, controller: 20 }),
            Some("/particles/hue")
        );
        assert!(state.learn_target.is_none(), "learn mode must clear itself");
        assert_eq!(state.revision, 1);
        // The message that taught the binding should not also jump the
        // value — otherwise learning snaps the parameter wherever the
        // knob happened to be.
        assert_eq!(reg.target(reg.id("/particles/hue").unwrap()), before);
    }

    /// The whole path, because the bug lived in the seam between
    /// resolving an event and writing it: `resolve` returned a position
    /// and the writer spread it across the range, so a button on a slot
    /// parameter could only ever reach the top.
    #[test]
    fn a_learned_pad_writes_its_own_slot_and_not_the_top_of_the_range() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        let fire = reg.id("/scene/fire").unwrap();
        shared.lock().unwrap().learn_target =
            Some(LearnTarget::value("/scene/fire", 3.0, "scene 3"));
        let mut d = Dispatcher::default();

        // Learn from note 36, full velocity.
        handle_message(&[0x90, 36, 127], &reg, &shared, &mut d);
        assert!(shared.lock().unwrap().learn_target.is_none());

        // Now press it for real. A plain note binding would land on 16.
        handle_message(&[0x90, 36, 127], &reg, &shared, &mut d);
        assert_eq!(reg.target(fire), 3.0);
        handle_message(&[0x80, 36, 0], &reg, &shared, &mut d);
        assert_eq!(reg.target(fire), 0.0, "release should rest at nothing-selected");
    }

    #[test]
    fn unknown_parameter_addresses_are_ignored() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        // A mapping file naming a parameter this build no longer has must
        // not panic or abort — it is simply inert.
        shared
            .lock()
            .unwrap()
            .map
            .bind(Source::ControlChange { channel: 0, controller: 7 }, "/gone/away");
        let mut d = Dispatcher::default();
        handle_message(&[0xB0, 7, 64], &reg, &shared, &mut d);
    }

    #[test]
    fn unmappable_traffic_is_ignored() {
        let reg = registry();
        let shared: SharedMidi = Arc::new(Mutex::new(MidiState::default()));
        let mut d = Dispatcher::default();
        // Clock floods at ~24 messages per beat; it must never set learn
        // state or reach the registry.
        for _ in 0..100 {
            handle_message(&[0xF8], &reg, &shared, &mut d);
        }
        assert!(shared.lock().unwrap().last_source.is_none());
    }

    #[test]
    fn map_file_round_trips_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("vizz-midi-test-{}", std::process::id()));
        let path = dir.join("midi.json");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(load_map(&path).unwrap().bindings.is_empty(), "missing file");

        let mut map = MidiMap::default();
        map.bind(Source::ControlChange { channel: 1, controller: 7 }, "/master/dim");
        // Also proves the parent directory is created.
        save_map(&path, &map).unwrap();
        assert_eq!(load_map(&path).unwrap(), map);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_map_file_reports_an_error_rather_than_panicking() {
        let dir = std::env::temp_dir().join(format!("vizz-midi-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("midi.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        assert!(load_map(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
