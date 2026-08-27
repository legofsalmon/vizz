//! Preset pictures, decoded once and kept on the GPU.
//!
//! [`vizz_mod::thumb`] stores a small PNG per look. Drawing one costs a
//! file read, a PNG decode and a texture upload — perfectly cheap once,
//! and ruinous at sixty times a second per button, which is what a naive
//! `read` inside the draw would do to a library of a hundred and sixty.
//!
//! So the handle is cached in the egui context, keyed by name. A look
//! with no picture caches that answer too: "there is none" has to be as
//! cheap to redraw as "here it is", or every preset that has never been
//! fired is a failed file read per frame for as long as the app is open.

/// A cache entry. `None` is a remembered absence, not a miss.
type Slot = Option<egui::TextureHandle>;

/// How many pictures may be decoded in one pass.
///
/// Opening the desk on a full library would otherwise decode and upload
/// a hundred and sixty PNGs inside one frame — a visible hitch at the
/// exact moment a performer is looking for something. Spread over a few
/// frames the library fills in under a second and nothing drops.
const BUDGET: usize = 4;

fn budget_id() -> egui::Id {
    egui::Id::new("preset-thumb-budget")
}

/// The picture of `name`, or `None` when there is not one *yet*.
///
/// `revision` is bumped by the app whenever it writes a thumbnail; it is
/// part of the key, so a look re-photographed mid-set redraws with the
/// new picture rather than the one egui already had.
///
/// A `None` can also mean "not decoded yet" — this frame's budget was
/// spent. Callers draw the fallback, and the picture arrives a frame or
/// two later. That is the whole reason the fallback is a real design
/// rather than an empty rectangle.
pub fn texture(ui: &egui::Ui, name: &str, revision: u64) -> Option<egui::TextureHandle> {
    let key = egui::Id::new(("preset-thumb", revision, name));
    if let Some(slot) = ui.data(|d| d.get_temp::<Slot>(key)) {
        return slot;
    }
    let pass = ui.ctx().cumulative_pass_nr();
    let used = match ui.data(|d| d.get_temp::<(u64, usize)>(budget_id())) {
        Some((at, used)) if at == pass => used,
        _ => 0,
    };
    if used >= BUDGET {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(budget_id(), (pass, used + 1)));

    let slot: Slot = vizz_mod::thumb::read(name).map(|t| {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [t.width as usize, t.height as usize],
            &t.rgba,
        );
        // Linear rather than nearest: a thumbnail is drawn at whatever
        // size the tile happens to be, which is never its own.
        ui.ctx()
            .load_texture(format!("preset-thumb-{name}"), image, egui::TextureOptions::LINEAR)
    });
    ui.data_mut(|d| d.insert_temp(key, slot.clone()));
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_picture() -> vizz_mod::thumb::Thumb {
        vizz_mod::thumb::Thumb {
            width: 4,
            height: 3,
            rgba: vec![255; 4 * 3 * 4],
        }
    }

    /// Run one egui pass and hand `f` a `Ui` to ask the cache with.
    fn pass<T>(ctx: &egui::Context, f: impl FnOnce(&egui::Ui) -> T) -> T {
        let mut out = None;
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        });
        egui::Area::new(egui::Id::new("thumb-test"))
            .fixed_pos([0.0, 0.0])
            .show(ctx, |ui| out = Some(f(ui)));
        let _ = ctx.end_pass();
        out.expect("the panel ran")
    }

    /// The point of the cache: the file is read once, not once a frame.
    ///
    /// Measured by deleting the file after the first look — a cache that
    /// re-read would then answer `None`, which is what happened before
    /// the handle was stored.
    #[test]
    fn a_picture_is_read_once_and_then_remembered() {
        let _guard = crate::project_bar::tests::scoped_config("thumb-cache");
        vizz_mod::thumb::save("night bus", &a_picture()).expect("saving");
        let ctx = egui::Context::default();
        assert!(pass(&ctx, |ui| texture(ui, "night bus", 0)).is_some(), "first look");
        vizz_mod::thumb::remove("night bus");
        assert!(
            pass(&ctx, |ui| texture(ui, "night bus", 0)).is_some(),
            "the picture was re-read from disk instead of remembered"
        );
    }

    /// An absence is remembered too, or every preset that has never been
    /// fired is a failed file read per frame forever.
    ///
    /// Measured the other way round: the file is written *after* the
    /// first look, and a cache that re-read would find it.
    #[test]
    fn a_missing_picture_is_remembered_as_missing() {
        let _guard = crate::project_bar::tests::scoped_config("thumb-cache-absent");
        let ctx = egui::Context::default();
        assert!(pass(&ctx, |ui| texture(ui, "warehouse", 0)).is_none(), "nothing there yet");
        vizz_mod::thumb::save("warehouse", &a_picture()).expect("saving");
        assert!(
            pass(&ctx, |ui| texture(ui, "warehouse", 0)).is_none(),
            "the absence was not cached — this is a file read per frame"
        );
    }

    /// A look re-photographed mid-set draws the new picture. Without the
    /// revision in the key, the old handle would outlive the file.
    #[test]
    fn a_new_revision_looks_again() {
        let _guard = crate::project_bar::tests::scoped_config("thumb-cache-revision");
        let ctx = egui::Context::default();
        assert!(pass(&ctx, |ui| texture(ui, "peak", 0)).is_none());
        vizz_mod::thumb::save("peak", &a_picture()).expect("saving");
        assert!(
            pass(&ctx, |ui| texture(ui, "peak", 1)).is_some(),
            "a bumped revision did not go back to the disk"
        );
    }

    /// Opening the desk on a full library must not decode all of it in
    /// one frame. The rest arrive over the following passes.
    #[test]
    fn one_pass_decodes_only_a_few() {
        let _guard = crate::project_bar::tests::scoped_config("thumb-cache-budget");
        let names: Vec<String> = (0..12).map(|n| format!("look {n}")).collect();
        for name in &names {
            vizz_mod::thumb::save(name, &a_picture()).expect("saving");
        }
        let ctx = egui::Context::default();
        let got = pass(&ctx, |ui| {
            names.iter().filter(|n| texture(ui, n, 0).is_some()).count()
        });
        assert_eq!(got, BUDGET, "{got} of 12 pictures were decoded in one pass");
        // And the library does fill in: four more each pass.
        let after = pass(&ctx, |ui| {
            names.iter().filter(|n| texture(ui, n, 0).is_some()).count()
        });
        assert_eq!(after, BUDGET * 2, "the second pass reached {after}, not {}", BUDGET * 2);
    }
}
