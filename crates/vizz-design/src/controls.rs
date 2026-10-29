//! The checkbox, the switch and the slider, drawn as the shared system draws
//! them: Material 3's shapes at the compact density's sizes.
//!
//! egui draws its own checkbox and slider from [`egui::Visuals`], and the
//! visuals cannot reach their shapes: a ticked box stays a grey square with a
//! line in it, and a slider stays a thin rail with a round knob. The design
//! system took Material's controls in 0.9.0 (components.md, Checkbox and
//! radio, Switch, Slider): a filled box with the tick cut out of it, a switch
//! whose handle grows and carries a tick when on, and a thick track filled up
//! to an upright bar with a gap either side of it. These are those, for egui.
//!
//! - [`Check`] is for a form you then apply. Something that takes effect at
//!   once is a [`Switch`]: say its label aloud with "on" after it.
//! - [`Slider`] keeps egui's slider underneath for the dragging, the keys and
//!   the screen reader, and draws over it; the value beside it is egui's own
//!   box, so typing a number still works.
//!
//! Hover and pressed lay the shared washes in a round halo (Material's state
//! layer); keyboard focus is the focus ring. Nothing grows under the pointer
//! but the switch's handle, which is under the finger and is the answer to
//! the press.

use egui::{
    Color32, CornerRadius, Id, NumExt as _, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    TextStyle, Ui, Vec2, Widget, WidgetInfo, WidgetText, WidgetType, emath, pos2, vec2,
};

use crate::tokens::{color::dark as role, duration, size::compact as size, space};
use crate::{ink, rgba, surface};

/// The colours the three controls are drawn in, from the shared roles.
mod paint {
    use super::*;

    pub const ACCENT: Color32 = rgba(role::accent::DEFAULT);
    pub const ACCENT_HOVER: Color32 = rgba(role::accent::HOVER);
    pub const ON_ACCENT: Color32 = rgba(role::accent::ON);
    pub const WELL: Color32 = rgba(role::surface::INSET);
    pub const EDGE: Color32 = rgba(role::line::CONTROL);
    pub const EDGE_HOVER: Color32 = rgba(role::ink::TERTIARY);
    pub const EDGE_OFF: Color32 = rgba(role::line::SUBTLE);
    pub const HANDLE: Color32 = rgba(role::ink::SECONDARY);
    pub const HANDLE_HOVER: Color32 = rgba(role::ink::PRIMARY);
    pub const DISABLED: Color32 = rgba(role::ink::DISABLED);
    pub const WASH_HOVER: Color32 = rgba(role::wash::HOVER);
    pub const WASH_PRESSED: Color32 = rgba(role::wash::PRESSED);
}

/// The 2px edge the system draws boxes, tracks and ticks with.
const EDGE: f32 = space::HALF;
/// A change in place: the mark fading in, the halo coming up, the handle.
const FAST: f32 = duration::FAST as f32 / 1000.0;

/// How the pointer and the keyboard are on a control this frame.
#[derive(Clone, Copy)]
struct Touch {
    enabled: bool,
    hovered: bool,
    pressed: bool,
    focused: bool,
}

impl Touch {
    fn of(ui: &Ui, r: &Response) -> Self {
        let enabled = ui.is_enabled();
        Self {
            enabled,
            hovered: enabled && r.hovered(),
            pressed: enabled && r.is_pointer_button_down_on(),
            focused: enabled && r.has_focus(),
        }
    }

    /// The halo's wash, faded at `fast` towards hover and at once to pressed.
    fn halo(self, ui: &Ui, id: Id) -> Color32 {
        let up = ui
            .ctx()
            .animate_bool_with_time(id.with("halo"), self.hovered, FAST);
        if self.pressed {
            paint::WASH_PRESSED
        } else {
            paint::WASH_HOVER.gamma_multiply(up)
        }
    }
}

/// Lays out a control's mark and its label after it, the label being part
/// of the target, and returns the mark's rect, the label and the response.
fn allocate(
    ui: &mut Ui,
    mark: Vec2,
    text: WidgetText,
    sense: Sense,
) -> (Rect, Option<std::sync::Arc<egui::Galley>>, Response) {
    let gap = space::STEP_2;
    let galley = (!text.is_empty()).then(|| {
        let wrap = ui.available_width() - mark.x - gap;
        text.into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            wrap,
            TextStyle::Button,
        )
    });
    let text_size = galley.as_ref().map_or(Vec2::ZERO, |g| g.size());
    let height = mark
        .y
        .max(text_size.y)
        .at_least(ui.spacing().interact_size.y);
    let width = mark.x
        + if galley.is_some() {
            gap + text_size.x
        } else {
            0.0
        };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    let mark = Rect::from_min_size(pos2(rect.left(), rect.center().y - mark.y / 2.0), mark);
    (mark, galley, response)
}

fn label(ui: &Ui, mark: Rect, galley: Option<std::sync::Arc<egui::Galley>>, enabled: bool) {
    if let Some(galley) = galley {
        let at = pos2(
            mark.right() + space::STEP_2,
            mark.center().y - galley.size().y / 2.0,
        );
        let ink = if enabled { ink::PRIMARY } else { ink::FAINT };
        ui.painter().galley(at, galley, ink);
    }
}

fn focus_ring(ui: &Ui, rect: Rect, corner: impl Into<CornerRadius>) {
    ui.painter().rect_stroke(
        rect.expand(EDGE),
        corner,
        Stroke::new(size::FOCUS_RING, surface::FOCUS),
        StrokeKind::Outside,
    );
}

/// A box or a track with its 2px edge. Filled in the accent, the edge is the
/// fill, and no stroke is drawn: an accent outline is the current item's mark.
fn edged(ui: &Ui, rect: Rect, corner: CornerRadius, fill: Color32, edge: Color32) {
    if fill == edge {
        ui.painter().rect_filled(rect, corner, fill);
    } else {
        ui.painter()
            .rect(rect, corner, fill, Stroke::new(EDGE, edge), StrokeKind::Inside);
    }
}

/// A tick drawn as the system draws it, two strokes meeting at the foot.
fn tick(ui: &Ui, centre: Pos2, side: f32, colour: Color32) {
    let s = side;
    let points = vec![
        centre + vec2(-0.32 * s, 0.02 * s),
        centre + vec2(-0.08 * s, 0.26 * s),
        centre + vec2(0.34 * s, -0.22 * s),
    ];
    ui.painter()
        .add(Shape::line(points, Stroke::new(EDGE, colour)));
}

/// A checkbox: one thing on or off, as part of a form you then apply.
///
/// `ui.add(Check::new(&mut on, "Include my licence"))` where egui's would be
/// `ui.checkbox(&mut on, …)`.
pub struct Check<'a> {
    checked: &'a mut bool,
    text: WidgetText,
}

impl<'a> Check<'a> {
    pub fn new(checked: &'a mut bool, text: impl Into<WidgetText>) -> Self {
        Self {
            checked,
            text: text.into(),
        }
    }
}

impl Widget for Check<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let side = (size::CONTROL_SM * 0.75).round();
        let text = self.text.text().to_owned();
        let (mark, galley, mut response) =
            allocate(ui, Vec2::splat(side), self.text, Sense::click());
        if response.clicked() {
            *self.checked = !*self.checked;
            response.mark_changed();
        }
        let on = *self.checked;
        response
            .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, &text));
        if !ui.is_rect_visible(response.rect) {
            return response;
        }

        let t = Touch::of(ui, &response);
        let corner = CornerRadius::same(crate::radius::CHIP as u8);
        if t.enabled {
            ui.painter()
                .circle_filled(mark.center(), side, t.halo(ui, response.id));
        }
        let shown = ui
            .ctx()
            .animate_bool_with_time(response.id.with("mark"), on, FAST);
        let (fill, edge) = match (t.enabled, on) {
            (true, true) => (paint::ACCENT, paint::ACCENT),
            (true, false) if t.hovered => (paint::WELL, paint::EDGE_HOVER),
            (true, false) => (paint::WELL, paint::EDGE),
            // Disabled and ticked: the box in the disabled ink, the tick cut
            // out of it in the well's colour.
            (false, true) => (paint::DISABLED, paint::DISABLED),
            (false, false) => (Color32::TRANSPARENT, paint::EDGE_OFF),
        };
        edged(ui, mark, corner, fill, edge);
        if on {
            let ink = if t.enabled {
                paint::ON_ACCENT
            } else {
                paint::WELL
            };
            tick(ui, mark.center(), side, ink.gamma_multiply(shown));
        }
        if t.focused {
            focus_ring(ui, mark, corner);
        }
        label(ui, mark, galley, t.enabled);
        response
    }
}

/// A switch: on or off, taking effect at once. Its label says what it does
/// when on, keyword first, and is never a question.
///
/// On is told by the tick, the side and the size of the handle, never by the
/// colour alone.
pub struct Switch<'a> {
    on: &'a mut bool,
    text: WidgetText,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool, text: impl Into<WidgetText>) -> Self {
        Self {
            on,
            text: text.into(),
        }
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let h = size::CONTROL_SM;
        let track = vec2((h * 1.625).round(), h);
        let text = self.text.text().to_owned();
        let (mark, galley, mut response) = allocate(ui, track, self.text, Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let on = *self.on;
        response
            .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, &text));
        if !ui.is_rect_visible(response.rect) {
            return response;
        }

        let t = Touch::of(ui, &response);
        // The handle slides and grows at `fast`, reduced motion or not: it is
        // under the finger, and it is the answer to the press.
        let at = ui
            .ctx()
            .animate_bool_with_time(response.id.with("side"), on, FAST);
        let held = ui
            .ctx()
            .animate_bool_with_time(response.id.with("held"), t.pressed, FAST);
        let pill = CornerRadius::same((h / 2.0) as u8);

        let (fill, edge) = match (t.enabled, on) {
            (true, true) => (paint::ACCENT, paint::ACCENT),
            (true, false) => (paint::WELL, paint::EDGE),
            (false, _) => (paint::WELL, paint::EDGE_OFF),
        };
        edged(ui, mark, pill, fill, edge);

        let inset = h / 2.0;
        let x = emath::lerp(mark.left() + inset..=mark.right() - inset, at);
        let centre = pos2(x, mark.center().y);
        // Half the track off, three quarters on, seven eighths held.
        let rest = emath::lerp(0.5..=0.75, at);
        let d = emath::lerp(rest..=0.875, held) * h;
        if t.enabled {
            ui.painter()
                .circle_filled(centre, h * 0.625, t.halo(ui, response.id));
        }
        let handle = match (t.enabled, on) {
            (true, true) => paint::ON_ACCENT,
            (true, false) if t.hovered => paint::HANDLE_HOVER,
            (true, false) => paint::HANDLE,
            (false, _) => paint::DISABLED,
        };
        ui.painter().circle_filled(centre, d / 2.0, handle);
        if on {
            // On and disabled: the tick is cut out of the handle in the
            // track's colour.
            let ink = if t.enabled {
                paint::ACCENT
            } else {
                paint::WELL
            };
            tick(ui, centre, d * 0.62, ink.gamma_multiply(at));
        }
        if t.focused {
            focus_ring(ui, mark, pill);
        }
        label(ui, mark, galley, t.enabled);
        response
    }
}

/// How a slider's value is written, as egui's `custom_formatter` takes it.
type Formatter<'a> = Box<dyn 'a + Fn(f64, std::ops::RangeInclusive<usize>) -> String>;

/// A value along a range, drawn as Material's slider: a thick track filled
/// in the accent up to the handle, an upright bar for the handle with the
/// track cut either side of it, and a dot saying where the end is.
///
/// It takes the parts of egui's builder that vizz uses, so a call changes
/// from `egui::Slider::new` to this and nothing else. Clamping is always on:
/// a value the track cannot show is one the eye cannot trust.
pub struct Slider<'a> {
    get_set: Box<dyn 'a + FnMut(Option<f64>) -> f64>,
    range: std::ops::RangeInclusive<f64>,
    integer: bool,
    text: WidgetText,
    suffix: String,
    step: Option<f64>,
    decimals: Option<usize>,
    show_value: bool,
    formatter: Option<Formatter<'a>>,
}

impl<'a> Slider<'a> {
    pub fn new<Num: emath::Numeric>(
        value: &'a mut Num,
        range: std::ops::RangeInclusive<Num>,
    ) -> Self {
        let range = range.start().to_f64()..=range.end().to_f64();
        Self {
            get_set: Box::new(move |v: Option<f64>| {
                if let Some(v) = v {
                    *value = Num::from_f64(v);
                }
                value.to_f64()
            }),
            range,
            integer: Num::INTEGRAL,
            text: WidgetText::default(),
            suffix: String::new(),
            step: None,
            decimals: None,
            show_value: true,
            formatter: None,
        }
    }

    /// The name after the value.
    pub fn text(mut self, text: impl Into<WidgetText>) -> Self {
        self.text = text.into();
        self
    }

    /// The unit after the number in the value box (" s", "x").
    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }

    pub fn step_by(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    pub fn fixed_decimals(mut self, decimals: usize) -> Self {
        self.decimals = Some(decimals);
        self
    }

    pub fn show_value(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }

    pub fn custom_formatter(
        mut self,
        f: impl 'a + Fn(f64, std::ops::RangeInclusive<usize>) -> String,
    ) -> Self {
        self.formatter = Some(Box::new(f));
        self
    }

    /// Kept so egui's spelling still reads: vizz's sliders always clamp.
    pub fn clamping(self, _: egui::SliderClamping) -> Self {
        self
    }
}

/// The track, the handle and the gap, at the compact density.
mod track {
    use super::*;
    /// Half the small control's height.
    pub const THICK: f32 = size::CONTROL_SM / 2.0;
    /// The handle: 4px wide, 2px while held. It stands the row's height,
    /// not the system's 24px target: the parameter list is 157 sliders, and
    /// a taller row is a quarter more scrolling to reach the instrument.
    pub const HANDLE_W: f32 = space::STEP_1;
    pub const HANDLE_HELD_W: f32 = space::HALF;
    /// The track is cut this far either side of the handle's centre.
    pub const CUT: f32 = space::STEP_1 + space::HALF + space::HALF;
    /// The dot at the end.
    pub const STOP: f32 = space::HALF;
}

impl Widget for Slider<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let range = self.range.clone();
        ui.horizontal(|ui| {
            // egui's slider underneath, drawn at no opacity: it keeps the
            // drag, the arrow keys, Home and End and the screen reader's
            // value. Its handle is told to be our handle's width, so the
            // value it reads from the pointer is the one drawn under it.
            let height = ui.spacing().interact_size.y.at_least(track::THICK);
            let half_handle = track::HANDLE_W / 2.0;
            let aspect = half_handle / (height / 2.5);
            let mut inner = egui::Slider::from_get_set(range.clone(), &mut self.get_set)
                .show_value(false)
                .clamping(egui::SliderClamping::Always)
                .handle_shape(egui::style::HandleShape::Rect {
                    aspect_ratio: aspect,
                })
                .integer_if(self.integer);
            if let Some(step) = self.step {
                inner = inner.step_by(step);
            }
            let rail = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size.y = height;
                    ui.multiply_opacity(0.0);
                    ui.add(inner)
                })
                .inner;
            let value = (self.get_set)(None);
            paint_track(ui, &rail, value, &range);

            if self.show_value {
                let mut drag = egui::DragValue::from_get_set(&mut self.get_set)
                    .range(range.clone())
                    .suffix(self.suffix.clone());
                if let Some(step) = self.step {
                    drag = drag.speed(step);
                } else {
                    // A point of drag is a point of track, as egui's own,
                    // which also sets how many decimals the box shows.
                    let width = f64::from(rail.rect.width().at_least(1.0));
                    drag = drag.speed((range.end() - range.start()) / width);
                }
                if let Some(d) = self.decimals {
                    drag = drag.fixed_decimals(d);
                }
                if self.integer {
                    drag = drag.fixed_decimals(0);
                }
                if let Some(f) = &self.formatter {
                    drag = drag.custom_formatter(f);
                }
                if ui.add(drag).changed() {
                    let mut rail = rail.clone();
                    rail.mark_changed();
                    return rail;
                }
            }
            if !self.text.is_empty() {
                ui.label(self.text);
            }
            rail
        })
        .inner
    }
}

trait IntegerIf {
    fn integer_if(self, integer: bool) -> Self;
}

impl IntegerIf for egui::Slider<'_> {
    fn integer_if(self, integer: bool) -> Self {
        if integer { self.integer() } else { self }
    }
}

/// Draws the slider over egui's invisible one.
fn paint_track(ui: &Ui, rail: &Response, value: f64, range: &std::ops::RangeInclusive<f64>) {
    if !ui.is_rect_visible(rail.rect) {
        return;
    }
    let t = Touch::of(ui, rail);
    let pressed = t.enabled && (rail.dragged() || rail.is_pointer_button_down_on());
    let rect = rail.rect;
    let (lo, hi) = (*range.start(), *range.end());
    let frac = if hi > lo {
        ((value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    // The handle travels where egui's does: inset by half its width.
    let half = track::HANDLE_W / 2.0;
    let x = emath::lerp(rect.left() + half..=rect.right() - half, frac);
    let cy = rect.center().y;
    let thick = track::THICK;
    let pill = CornerRadius::same((thick / 2.0) as u8);

    let (active, rest) = if !t.enabled {
        (paint::DISABLED, paint::EDGE_OFF)
    } else if t.hovered || pressed {
        (paint::ACCENT_HOVER, paint::EDGE)
    } else {
        (paint::ACCENT, paint::EDGE)
    };
    let p = ui.painter();
    let left_end = x - track::CUT;
    if left_end > rect.left() {
        let r = Rect::from_min_max(
            pos2(rect.left(), cy - thick / 2.0),
            pos2(left_end, cy + thick / 2.0),
        );
        p.rect_filled(r, pill, active);
    }
    let right_start = x + track::CUT;
    if right_start < rect.right() {
        let r = Rect::from_min_max(
            pos2(right_start, cy - thick / 2.0),
            pos2(rect.right(), cy + thick / 2.0),
        );
        p.rect_filled(r, pill, rest);
    }
    // The dot at the far end, unless the handle has reached it.
    let stop = pos2(rect.right() - thick / 2.0, cy);
    if right_start < stop.x - track::STOP {
        p.circle_filled(stop, track::STOP, active);
    }
    // Held, the handle thins so the value under it shows.
    let w = if pressed {
        track::HANDLE_HELD_W
    } else {
        track::HANDLE_W
    };
    let handle = Rect::from_center_size(pos2(x, cy), vec2(w, rect.height()));
    p.rect_filled(handle, CornerRadius::same((w / 2.0) as u8), active);
    if t.focused {
        focus_ring(ui, rect, pill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnMut(&mut Ui)) {
        let ctx = egui::Context::default();
        let mut f = f;
        ctx.begin_pass(egui::RawInput::default());
        egui::Area::new(Id::new("controls")).show(&ctx, |ui| f(ui));
        let _ = ctx.end_pass();
    }

    /// The track, the handle and the gap are the system's sizes.
    #[test]
    fn the_handle_is_the_systems_size() {
        assert_eq!(track::THICK, 10.0);
        assert_eq!(track::HANDLE_W, 4.0);
        assert_eq!(track::CUT, 8.0);
    }

    /// Every builder vizz calls is here, and the controls lay out.
    #[test]
    fn the_controls_lay_out() {
        run(|ui| {
            let mut on = true;
            let mut v = 0.5f32;
            let mut n = 3u32;
            ui.add(Check::new(&mut on, "include my licence"));
            ui.add(Switch::new(&mut on, "send over NDI"));
            ui.add(
                Slider::new(&mut v, 0.0..=1.0)
                    .text("opacity")
                    .clamping(egui::SliderClamping::Always),
            );
            ui.add(Slider::new(&mut v, 0.0..=30.0).suffix(" s"));
            ui.add(Slider::new(&mut n, 1..=32).step_by(1.0));
            ui.add(
                Slider::new(&mut v, -1.0..=1.0)
                    .fixed_decimals(2)
                    .show_value(true),
            );
            ui.add(Slider::new(&mut v, 0.0..=4.0).custom_formatter(|v, _| format!("{v:.0}")));
        });
    }
}
