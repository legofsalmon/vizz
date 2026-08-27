//! Photographing a look for its tile.
//!
//! A preset's picture is the master output at the moment the look was
//! saved or last fired — see [`vizz_mod::thumb`] for what is stored and
//! [`vizz_ui::thumbs`] for how a tile finds it. This is the half that
//! takes it: one asynchronous readback of the eight-bit master, shrunk on
//! the CPU and written to disk.
//!
//! It rides the same [`vizz_io::readback::ReadbackRing`] the recorder and
//! the NDI sender use, and for the same reason: the render thread must
//! never wait on the GPU. A picture that arrives two frames late is a
//! picture; a stalled frame is a dropped one.
//!
//! The ring is built for the shot and dropped with it. A staging buffer
//! the size of the master is several megabytes, and this takes at most
//! one picture every second or so — carrying that buffer permanently for
//! something that idle would be the wrong trade, and it would also have
//! to be rebuilt on every resize.

use std::time::{Duration, Instant};

/// How long a recalled look is given to settle before it is photographed.
///
/// A recall does not change the picture, it starts a change: `/shape/mode`
/// is smoothed, the particles are a simulation, and a photograph taken on
/// the next frame is of the look you just left. A second is comfortably
/// past the longest parameter smoothing in the registry and past the
/// visible part of a morph.
const SETTLE: Duration = Duration::from_millis(1200);

/// A picture asked for, and the earliest it may be taken.
struct Wanted {
    name: String,
    at: Instant,
}

#[derive(Default)]
pub struct Shutter {
    wanted: Option<Wanted>,
    /// A readback in flight, and who it is of.
    inflight: Option<(String, vizz_io::readback::ReadbackRing)>,
    /// Bumped whenever a picture lands, so the tiles reload it rather
    /// than keeping the texture egui already had.
    pub revision: u64,
}

impl Shutter {
    /// Photograph `name` from the next frame.
    ///
    /// For a look being *saved*: what is on screen is the thing being
    /// saved, so there is nothing to wait for.
    pub fn now(&mut self, name: &str) {
        self.want(name, Instant::now());
    }

    /// Photograph `name` once the look it recalled has settled.
    ///
    /// Only if it has no picture yet: a look that has been photographed
    /// keeps the picture it was saved with, and re-shooting on every
    /// recall would quietly replace it with whatever the parameters had
    /// been dragged to since.
    pub fn if_missing(&mut self, name: &str) {
        if vizz_mod::thumb::exists(name) {
            return;
        }
        self.want(name, Instant::now() + SETTLE);
    }

    /// The newest request wins.
    ///
    /// Two recalls a second apart would otherwise photograph the second
    /// look and file it under the first — the one case where a picture
    /// is worse than no picture, because it is confidently wrong.
    fn want(&mut self, name: &str, at: Instant) {
        self.wanted = Some(Wanted { name: name.to_string(), at });
    }

    /// Called once a frame, after the master has been published.
    ///
    /// `texture` must be the eight-bit master — the same one the senders
    /// and the recorder are given. Never blocks: the shot is enqueued on
    /// one frame and collected on a later one.
    pub fn tick(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) {
        self.collect();
        if self.inflight.is_some() {
            return;
        }
        let Some(wanted) = &self.wanted else {
            return;
        };
        if Instant::now() < wanted.at {
            return;
        }
        let Wanted { name, .. } = self.wanted.take().expect("just checked");
        let size = texture.size();
        // One slot: there is one shot in flight at a time by construction.
        let mut ring =
            match vizz_io::readback::ReadbackRing::new(device, size.width, size.height, 1) {
                Ok(ring) => ring,
                Err(e) => {
                    log::warn!("could not photograph {name}: {e:#}");
                    return;
                }
            };
        if !ring.capture(device, queue, texture) {
            log::warn!("could not photograph {name}: the readback was refused");
            return;
        }
        self.inflight = Some((name, ring));
    }

    /// Take the picture off the GPU if it has arrived, and write it.
    fn collect(&mut self) {
        let Some((_, ring)) = &mut self.inflight else {
            return;
        };
        let Some(frame) = ring.take_ready() else {
            return;
        };
        let (name, _) = self.inflight.take().expect("just borrowed");
        let made = frame.with_bytes(|bytes| {
            vizz_mod::thumb::from_bgra(bytes, frame.width, frame.height, frame.stride)
        });
        match made {
            Ok(Some(thumb)) => match vizz_mod::thumb::save(&name, &thumb) {
                // A picture is a convenience, so every failure here is a
                // log line and nothing more. Losing one must not put a
                // notice on a screen somebody is performing from, and it
                // certainly must not take the look with it.
                Ok(()) => {
                    log::info!("photographed preset {name} at {}x{}", thumb.width, thumb.height);
                    self.revision += 1;
                }
                Err(e) => log::warn!("could not write the picture of {name}: {e:#}"),
            },
            Ok(None) => log::warn!("the picture of {name} did not fit its buffer"),
            Err(e) => log::warn!("could not read the picture of {name}: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A look that already has a picture keeps it.
    ///
    /// Otherwise every recall silently replaces the saved picture with
    /// whatever the parameters had been dragged to since — the tile stops
    /// being a picture of the look and becomes a picture of the last time
    /// you happened to press it.
    #[test]
    fn a_look_that_has_a_picture_is_not_re_shot() {
        let (_guard, _tmp) = scoped("thumbshot-existing");
        let mut camera = Shutter::default();
        camera.if_missing("night bus");
        assert!(camera.wanted.is_some(), "a look with no picture was not queued");
        vizz_mod::thumb::save(
            "night bus",
            &vizz_mod::thumb::Thumb { width: 2, height: 2, rgba: vec![255; 16] },
        )
        .unwrap();
        let mut camera = Shutter::default();
        camera.if_missing("night bus");
        assert!(camera.wanted.is_none(), "a look with a picture was queued anyway");
    }

    /// The newest request wins. A picture of the wrong look, filed
    /// confidently under a name, is worse than no picture at all.
    #[test]
    fn a_second_recall_replaces_the_first_shot() {
        let (_guard, _tmp) = scoped("thumbshot-replace");
        let mut camera = Shutter::default();
        camera.if_missing("first");
        camera.if_missing("second");
        assert_eq!(
            camera.wanted.as_ref().map(|w| w.name.as_str()),
            Some("second"),
            "the first look would have been photographed as the second"
        );
    }

    /// Saving photographs immediately; recalling waits for the morph.
    #[test]
    fn a_save_does_not_wait_and_a_recall_does() {
        let (_guard, _tmp) = scoped("thumbshot-timing");
        let mut camera = Shutter::default();
        camera.now("saved");
        assert!(camera.wanted.as_ref().unwrap().at <= Instant::now());
        camera.if_missing("recalled");
        assert!(
            camera.wanted.as_ref().unwrap().at > Instant::now(),
            "a recalled look would be photographed mid-morph"
        );
    }

    /// Config storage pointed somewhere private, so these cannot see the
    /// developer's own library. Serialised: the environment is per
    /// process.
    fn scoped(tag: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let guard = LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("vizz-app-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the mutex makes this the only thread touching the
        // environment for as long as the guard is held.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        (guard, dir)
    }
}
