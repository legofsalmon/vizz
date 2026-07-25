//! The control panel content.
//!
//! Every slider is generated from [`ParamRegistry`]'s own metadata rather
//! than hand-written, so adding a parameter to the app's table gives it a
//! control automatically and the GUI can never drift out of sync with the
//! OSC surface. The panel writes targets exactly like the OSC listener
//! does — it gets no privileged access to the renderer.

use vizz_health::HealthSnapshot;
use vizz_params::ParamRegistry;

/// Status of one video output, as shown in the panel.
pub struct OutputStatus {
    pub name: String,
    pub live: bool,
}

/// Everything the panel displays that it cannot read from the registry.
pub struct PanelState {
    pub health: Option<HealthSnapshot>,
    pub outputs: Vec<OutputStatus>,
    /// Recent frame times in ms, oldest first, for the sparkline.
    pub frame_times_ms: Vec<f32>,
    pub frame_budget_ms: f32,
}

pub fn draw(ctx: &egui::Context, registry: &ParamRegistry, state: &PanelState) {
    egui::Window::new("vizz")
        .default_pos([12.0, 12.0])
        .default_width(320.0)
        .resizable(true)
        .show(ctx, |ui| {
            health_section(ui, state);
            ui.separator();
            outputs_section(ui, state);
            ui.separator();
            params_section(ui, registry);
            ui.separator();
            ui.small("Tab toggles this panel · Esc quits");
        });
}

fn health_section(ui: &mut egui::Ui, state: &PanelState) {
    let Some(h) = &state.health else {
        ui.label("Collecting health data…");
        return;
    };

    // Colour the headline by whether we are actually holding the budget —
    // this is the number that matters mid-set, readable at a glance.
    let over = h.frame_avg_ms > state.frame_budget_ms;
    let color = if over {
        egui::Color32::from_rgb(255, 120, 90)
    } else {
        egui::Color32::from_rgb(120, 220, 150)
    };
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new(format!("{:.0} fps", h.fps)).color(color));
        ui.label(
            egui::RichText::new(format!("{:.2} ms avg", h.frame_avg_ms))
                .color(color)
                .small(),
        );
    });

    sparkline(ui, &state.frame_times_ms, state.frame_budget_ms);

    egui::Grid::new("health-grid").num_columns(2).show(ui, |ui| {
        ui.label("p95 / p99");
        ui.label(format!("{:.2} / {:.2} ms", h.frame_p95_ms, h.frame_p99_ms));
        ui.end_row();
        ui.label("worst");
        ui.label(format!("{:.2} ms", h.frame_worst_ms));
        ui.end_row();
        ui.label("over budget");
        ui.label(format!("{:.1}% ({} total)", h.over_budget_window_pct, h.over_budget_total));
        ui.end_row();
        ui.label("memory / cpu");
        ui.label(format!(
            "{} · {}",
            h.rss_mib.map(|m| format!("{m:.0} MiB")).unwrap_or_else(|| "n/a".into()),
            h.cpu_pct.map(|c| format!("{c:.0}%")).unwrap_or_else(|| "n/a".into()),
        ));
        ui.end_row();
        ui.label("frames");
        ui.label(format!("{}", h.frames_total));
        ui.end_row();
    });
}

/// Frame-time history with the budget drawn as a reference line, so a
/// spike is visible as shape rather than as a number that already passed.
fn sparkline(ui: &mut egui::Ui, samples: &[f32], budget_ms: f32) {
    let height = 40.0;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(120));

    if samples.is_empty() {
        return;
    }
    // Scale to the budget or the worst sample, whichever is larger, so the
    // budget line stays on screen and spikes are not clipped.
    let peak = samples.iter().copied().fold(budget_ms, f32::max).max(0.001);

    let budget_y = rect.bottom() - (budget_ms / peak) * rect.height();
    painter.line_segment(
        [egui::pos2(rect.left(), budget_y), egui::pos2(rect.right(), budget_y)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 90, 110)),
    );

    let step = rect.width() / samples.len().max(1) as f32;
    let points: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(i, &ms)| {
            egui::pos2(
                rect.left() + i as f32 * step,
                rect.bottom() - (ms / peak).clamp(0.0, 1.0) * rect.height(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 190, 255)),
    ));
}

fn outputs_section(ui: &mut egui::Ui, state: &PanelState) {
    ui.label(egui::RichText::new("Outputs").strong());
    if state.outputs.is_empty() {
        ui.small("none active — preview only");
        return;
    }
    for out in &state.outputs {
        ui.horizontal(|ui| {
            // Painted rather than a glyph: egui's default font has no
            // U+25CF, so a text bullet renders as a missing-glyph box.
            let color = if out.live {
                egui::Color32::from_rgb(120, 220, 150)
            } else {
                egui::Color32::from_gray(120)
            };
            let size = egui::vec2(10.0, 10.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let center = rect.center();
            if out.live {
                ui.painter().circle_filled(center, 4.0, color);
            } else {
                ui.painter().circle_stroke(center, 4.0, egui::Stroke::new(1.0, color));
            }
            ui.label(&out.name);
        });
    }
}

fn params_section(ui: &mut egui::Ui, registry: &ParamRegistry) {
    ui.label(egui::RichText::new("Parameters").strong());
    for (id, def) in registry.iter() {
        let mut value = registry.target(id);
        // Strip the leading '/' and show the address as the label, so the
        // panel doubles as live documentation of the OSC surface.
        let label = def.addr.trim_start_matches('/');
        let response = ui.add(
            egui::Slider::new(&mut value, def.min..=def.max)
                .text(label)
                .clamping(egui::SliderClamping::Always),
        );
        if response.changed() {
            registry.set(id, value);
        }
        // Right-click restores the default: the fastest way out of a mess
        // mid-set, and it costs nothing to support.
        if response.secondary_clicked() {
            registry.set(id, def.default);
        }
    }
    ui.small("right-click a slider to reset it");
}
