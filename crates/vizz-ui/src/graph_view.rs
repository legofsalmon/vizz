//! The node canvas: a pannable, zoomable editor for the modulation graph.
//!
//! Hit-testing is done by hand rather than by allocating an egui widget per
//! node. Nodes overlap, sit on a transformed surface, and have small ports
//! that must win against the much larger body behind them — a stack of
//! allocated rects gets that ordering wrong, and the failure mode is a wire
//! drag that silently starts a node move instead.
//!
//! The geometry is deliberately separate from the drawing (see
//! [`Layout`]), because the parts that break are coordinate transforms and
//! port positions, and those are testable without a display. Drawing is the
//! part you have to look at.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use vizz_mod::graph::{Category, NodeGraph, NodeId, NodeKind, catalog};
use vizz_params::ParamRegistry;

/// Node size in graph units. Fixed, so a patch's shape is stable across
/// zoom levels and a saved layout means the same thing on any screen.
const NODE_W: f32 = 148.0;
const TITLE_H: f32 = 22.0;
const ROW_H: f32 = 15.0;
const PORT_R: f32 = 4.5;
/// Generous relative to the drawn radius: ports are the smallest target on
/// the canvas and the most costly to miss mid-set.
const PORT_HIT_R: f32 = 9.0;
/// Height reserved for the selected-node inspector below the canvas.
const INSPECTOR_H: f32 = 62.0;
/// Width of the node palette strip.
const PALETTE_W: f32 = 132.0;

/// Maps between graph space and screen space, and knows where everything
/// sits. Pure geometry — no egui state, no drawing.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub origin: Pos2,
    pub pan: Vec2,
    pub zoom: f32,
}

impl Layout {
    pub fn to_screen(&self, p: [f32; 2]) -> Pos2 {
        self.origin + (vec2(p[0], p[1]) + self.pan) * self.zoom
    }

    pub fn to_graph(&self, p: Pos2) -> [f32; 2] {
        let v = (p - self.origin) / self.zoom - self.pan;
        [v.x, v.y]
    }

    pub fn node_rect(&self, node_pos: [f32; 2], inputs: usize) -> Rect {
        let h = TITLE_H + ROW_H * (inputs.max(1) as f32) + 10.0;
        Rect::from_min_size(self.to_screen(node_pos), vec2(NODE_W, h) * self.zoom)
    }

    /// Output port: right edge, level with the title bar.
    pub fn output_pos(&self, node_pos: [f32; 2], inputs: usize) -> Pos2 {
        let r = self.node_rect(node_pos, inputs);
        pos2(r.right(), r.top() + TITLE_H * 0.5 * self.zoom)
    }

    /// Input ports: left edge, one per row below the title.
    pub fn input_pos(&self, node_pos: [f32; 2], inputs: usize, port: usize) -> Pos2 {
        let r = self.node_rect(node_pos, inputs);
        pos2(
            r.left(),
            r.top() + (TITLE_H + ROW_H * (port as f32 + 0.5)) * self.zoom,
        )
    }

    /// Topmost node under a point. Later nodes draw on top, so the search
    /// runs backwards — otherwise clicking overlapping nodes grabs the one
    /// underneath.
    pub fn node_at(&self, graph: &NodeGraph, p: Pos2) -> Option<NodeId> {
        graph
            .nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, n)| self.node_rect(n.pos, n.kind.inputs()).contains(p))
            .map(|(i, _)| NodeId(i))
    }

    /// Port under a point, as (node, port). `None` port means the output.
    pub fn port_at(&self, graph: &NodeGraph, p: Pos2) -> Option<(NodeId, Option<usize>)> {
        let r = PORT_HIT_R * self.zoom.clamp(0.5, 1.5);
        for (i, n) in graph.nodes.iter().enumerate().rev() {
            let inputs = n.kind.inputs();
            if self.output_pos(n.pos, inputs).distance(p) <= r {
                return Some((NodeId(i), None));
            }
            for port in 0..inputs {
                if self.input_pos(n.pos, inputs, port).distance(p) <= r {
                    return Some((NodeId(i), Some(port)));
                }
            }
        }
        None
    }
}

/// What the user is currently dragging.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    None,
    /// Moving a node; the offset keeps the grab point under the cursor.
    Node(NodeId, Vec2),
    Pan,
    /// Pulling a wire out of an output.
    WireFrom(NodeId),
    /// Pulling an existing wire off an input, to move or drop it.
    WireTo(NodeId, usize),
}

/// Persistent canvas state. Lives with the app, not the graph — pan and
/// zoom are where *you* are looking, not part of the patch.
pub struct GraphView {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Option<NodeId>,
    drag: Drag,
    /// Where a right-click opened the add menu, in graph space, so a node
    /// appears where it was asked for rather than at the origin.
    add_at: [f32; 2],
    /// Current patch name, for save/load.
    pub patch_name: String,
    /// Transient feedback ("saved", "load failed: …") and when it was set,
    /// so it fades rather than sitting there misleading you later.
    status: Option<(String, f64)>,
    pub show_palette: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            pan: vec2(40.0, 40.0),
            zoom: 1.0,
            selected: None,
            drag: Drag::None,
            add_at: [0.0, 0.0],
            patch_name: String::new(),
            status: None,
            show_palette: true,
        }
    }
}

impl GraphView {
    /// Start at a given zoom. Struct-update syntax cannot reach the private
    /// interaction state, and that state should stay private — a caller
    /// setting a half-finished drag would be a bug.
    pub fn with_zoom(zoom: f32) -> Self {
        Self { zoom, ..Default::default() }
    }

    pub fn layout(&self, origin: Pos2) -> Layout {
        Layout { origin, pan: self.pan, zoom: self.zoom }
    }

    /// Draw and interact. Returns true if the graph changed structurally,
    /// so the caller can persist the patch.
    pub fn show(&mut self, ui: &mut egui::Ui, graph: &mut NodeGraph, registry: &ParamRegistry) -> bool {
        let mut changed = false;
        changed |= self.toolbar(ui, graph);
        ui.separator();
        // Palette and canvas side by side. Width is taken from the parent
        // before the canvas allocates, for the same reason the inspector
        // reserves its strip: whichever allocates first would otherwise
        // take everything.
        let full = ui.available_size();
        let canvas_w = if self.show_palette { full.x - PALETTE_W - 8.0 } else { full.x };
        let mut inner_changed = false;
        ui.horizontal_top(|ui| {
            // Both regions must be explicitly top-down: `allocate_ui`
            // inherits the parent's layout, and inside a horizontal parent
            // that lays the palette out left-to-right, consuming the full
            // width and leaving the canvas nothing.
            let down = egui::Layout::top_down(egui::Align::Min);
            if self.show_palette {
                ui.allocate_ui_with_layout(vec2(PALETTE_W, full.y), down, |ui| {
                    inner_changed |= self.palette(ui, graph);
                });
            }
            ui.allocate_ui_with_layout(vec2(canvas_w.max(120.0), full.y), down, |ui| {
                inner_changed |= self.canvas(ui, graph, registry);
            });
        });
        changed | inner_changed
    }

    /// Patch name, save/load, and the palette toggle.
    fn toolbar(&mut self, ui: &mut egui::Ui, graph: &mut NodeGraph) -> bool {
        use vizz_mod::library;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.patch_name)
                    .hint_text("patch name")
                    .desired_width(140.0),
            );
            if ui.button("save").clicked() {
                match library::save(&self.patch_name, graph) {
                    Ok(p) => {
                        // Show the sanitised name back: a patch saved as
                        // "café/bar" lands as "caf__bar", and silently
                        // renaming it would make it unfindable later.
                        let saved = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                        self.patch_name = saved.clone();
                        self.set_status(ui, format!("saved “{saved}”"));
                    }
                    Err(e) => self.set_status(ui, format!("save failed: {e}")),
                }
            }
            let patches = library::list();
            egui::ComboBox::from_id_salt("load-patch")
                .selected_text("load")
                .width(90.0)
                .show_ui(ui, |ui| {
                    if patches.is_empty() {
                        ui.label(egui::RichText::new("no saved patches").small());
                    }
                    for name in &patches {
                        if ui.selectable_label(false, name).clicked() {
                            match library::load(name) {
                                Ok(g) => {
                                    *graph = g;
                                    self.patch_name = name.clone();
                                    self.selected = None;
                                    changed = true;
                                }
                                Err(e) => self.set_status(ui, format!("load failed: {e}")),
                            }
                        }
                    }
                });
            if ui.button("new").on_hover_text("clear the graph").clicked() {
                *graph = NodeGraph::default();
                self.selected = None;
                self.patch_name.clear();
                changed = true;
            }
            ui.checkbox(&mut self.show_palette, "palette");

            if let Some((msg, at)) = &self.status {
                // Fade after a few seconds: stale feedback next to a
                // changed graph reads as a fresh result.
                let age = ui.ctx().input(|i| i.time) - at;
                if age < 4.0 {
                    ui.label(egui::RichText::new(msg).small().color(
                        Color32::from_rgb(150, 200, 160).gamma_multiply(
                            (1.0 - (age / 4.0) as f32).clamp(0.15, 1.0),
                        ),
                    ));
                }
            }
        });
        changed
    }

    fn set_status(&mut self, ui: &egui::Ui, msg: String) {
        self.status = Some((msg, ui.ctx().input(|i| i.time)));
    }

    /// Every node kind, grouped. The canvas add-menu and this read the same
    /// catalogue, so a new kind appears in both without being registered
    /// twice — and this one makes the operators discoverable rather than
    /// hidden behind a right-click nobody thinks to try.
    fn palette(&mut self, ui: &mut egui::Ui, graph: &mut NodeGraph) -> bool {
        let mut changed = false;
        egui::ScrollArea::vertical().id_salt("palette").show(ui, |ui| {
            for group in [Category::Source, Category::Operator, Category::Sink] {
                ui.label(
                    egui::RichText::new(group_label(group))
                        .small()
                        .strong()
                        .color(category_color(group).gamma_multiply(1.6)),
                );
                for (cat, name, kind) in catalog() {
                    if cat != group {
                        continue;
                    }
                    if ui
                        .add_sized([PALETTE_W - 14.0, 18.0], egui::Button::new(name))
                        .on_hover_text("add to the canvas")
                        .clicked()
                    {
                        // Drop new nodes near the middle of where the user
                        // is looking rather than at the graph origin, which
                        // may be off-screen entirely.
                        let at = [-self.pan.x + 160.0, -self.pan.y + 80.0];
                        let id = graph.add(kind, at);
                        self.selected = Some(id);
                        changed = true;
                    }
                }
                ui.add_space(4.0);
            }
        });
        changed
    }

    fn canvas(&mut self, ui: &mut egui::Ui, graph: &mut NodeGraph, registry: &ParamRegistry) -> bool {
        let mut changed = false;
        // Reserve the inspector's strip before the canvas claims the space.
        // Allocating everything to the canvas leaves the inspector nothing
        // to draw into, and it silently never appears.
        let has_selection = self
            .selected
            .is_some_and(|s| s.0 < graph.nodes.len());
        let inspector_h = if has_selection { INSPECTOR_H } else { 0.0 };
        let canvas_size = vec2(
            ui.available_width(),
            (ui.available_height() - inspector_h).max(80.0),
        );
        let (rect, response) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 26, 30));

        let lay = self.layout(rect.min);
        self.draw_grid(&painter, rect, &lay);

        let pointer = ui.ctx().pointer_latest_pos().filter(|p| rect.contains(*p));

        // Zoom about the cursor, so the thing under the pointer stays put.
        if response.hovered() {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 && let Some(p) = pointer {
                let before = lay.to_graph(p);
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.35, 2.5);
                let after = self.layout(rect.min).to_graph(p);
                self.pan += vec2(after[0] - before[0], after[1] - before[1]);
            }
        }
        let lay = self.layout(rect.min);

        self.begin_drag(&response, &lay, graph, pointer);
        changed |= self.update_drag(&response, &lay, graph, pointer, ui);

        self.draw_wires(&painter, &lay, graph, pointer);
        self.draw_nodes(&painter, &lay, graph);

        changed |= self.add_menu(&response, graph, &lay, pointer);
        changed |= self.handle_delete(ui, graph);
        self.inspector(ui, graph, registry, &mut changed);
        changed
    }

    fn begin_drag(
        &mut self,
        response: &egui::Response,
        lay: &Layout,
        graph: &NodeGraph,
        pointer: Option<Pos2>,
    ) {
        if !response.drag_started() || self.drag != Drag::None {
            return;
        }
        let Some(p) = pointer else { return };
        // Ports before bodies: a port sits on top of the node it belongs
        // to, and grabbing the body instead would move the node when the
        // user meant to pull a wire.
        if let Some((id, port)) = lay.port_at(graph, p) {
            self.drag = match port {
                None => Drag::WireFrom(id),
                Some(port) => Drag::WireTo(id, port),
            };
            return;
        }
        if let Some(id) = lay.node_at(graph, p) {
            let node_screen = lay.to_screen(graph.nodes[id.0].pos);
            self.drag = Drag::Node(id, p - node_screen);
            self.selected = Some(id);
            return;
        }
        self.selected = None;
        self.drag = Drag::Pan;
    }

    fn update_drag(
        &mut self,
        response: &egui::Response,
        lay: &Layout,
        graph: &mut NodeGraph,
        pointer: Option<Pos2>,
        ui: &egui::Ui,
    ) -> bool {
        let mut changed = false;
        match self.drag {
            Drag::Pan => self.pan += response.drag_delta() / lay.zoom,
            Drag::Node(id, grab) => {
                if let Some(p) = pointer
                    && id.0 < graph.nodes.len()
                {
                    graph.nodes[id.0].pos = lay.to_graph(p - grab);
                }
            }
            _ => {}
        }

        if !response.drag_stopped() {
            return changed;
        }
        let dropped = pointer.and_then(|p| lay.port_at(graph, p));
        match (self.drag, dropped) {
            // Output dragged onto an input.
            (Drag::WireFrom(from), Some((to, Some(port)))) => {
                if graph.would_cycle(from, to) {
                    // Refuse rather than accept and disable: a wire that
                    // appears and then goes dead is worse than one that
                    // never lands.
                    log::debug!("refused a connection that would cycle");
                } else {
                    graph.connect(from, to, port);
                    changed = true;
                }
            }
            // Input dragged onto an output — the same wire, other way up.
            (Drag::WireTo(to, port), Some((from, None))) => {
                if !graph.would_cycle(from, to) {
                    graph.connect(from, to, port);
                    changed = true;
                }
            }
            // An input dragged to nowhere means "unplug this".
            (Drag::WireTo(to, port), _) => {
                graph.disconnect(to, port);
                changed = true;
            }
            _ => {}
        }
        let _ = ui;
        self.drag = Drag::None;
        changed
    }

    fn handle_delete(&mut self, ui: &egui::Ui, graph: &mut NodeGraph) -> bool {
        let Some(id) = self.selected else { return false };
        let pressed = ui.ctx().input(|i| {
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
        });
        if !pressed {
            return false;
        }
        graph.remove(id);
        self.selected = None;
        true
    }

    fn add_menu(
        &mut self,
        response: &egui::Response,
        graph: &mut NodeGraph,
        lay: &Layout,
        pointer: Option<Pos2>,
    ) -> bool {
        if response.secondary_clicked() && let Some(p) = pointer {
            self.add_at = lay.to_graph(p);
        }
        let mut added = false;
        response.context_menu(|ui| {
            ui.set_min_width(150.0);
            for group in [Category::Source, Category::Operator, Category::Sink] {
                ui.label(egui::RichText::new(group_label(group)).small().strong());
                for (cat, name, kind) in catalog() {
                    if cat != group {
                        continue;
                    }
                    if ui.button(name).clicked() {
                        graph.add(kind, self.add_at);
                        added = true;
                        ui.close();
                    }
                }
                ui.separator();
            }
        });
        added
    }

    fn draw_grid(&self, p: &egui::Painter, rect: Rect, lay: &Layout) {
        let step = 26.0 * lay.zoom;
        if step < 6.0 {
            return; // Too dense to read; drawing it is just noise.
        }
        let off = (lay.pan * lay.zoom).to_pos2();
        let start = pos2(
            rect.left() + off.x.rem_euclid(step),
            rect.top() + off.y.rem_euclid(step),
        );
        let mut y = start.y;
        while y < rect.bottom() {
            let mut x = start.x;
            while x < rect.right() {
                p.circle_filled(pos2(x, y), 1.0, Color32::from_rgb(44, 48, 54));
                x += step;
            }
            y += step;
        }
    }

    fn draw_wires(&self, p: &egui::Painter, lay: &Layout, graph: &NodeGraph, pointer: Option<Pos2>) {
        for e in &graph.edges {
            if e.from.0 >= graph.nodes.len() || e.to.0 >= graph.nodes.len() {
                continue;
            }
            let from = &graph.nodes[e.from.0];
            let to = &graph.nodes[e.to.0];
            let a = lay.output_pos(from.pos, from.kind.inputs());
            let b = lay.input_pos(to.pos, to.kind.inputs(), e.port);
            // A wire into a cycle is drawn red: the graph excludes those
            // nodes, and silently dead wiring is the worst outcome.
            let live = !graph.cycle_nodes().contains(&e.to.0);
            let color = if live {
                Color32::from_rgb(96, 122, 150)
            } else {
                Color32::from_rgb(190, 80, 70)
            };
            bezier(p, a, b, color, 1.6 * lay.zoom.max(0.5));
        }

        // The wire currently being pulled.
        if let Some(cursor) = pointer {
            let anchor = match self.drag {
                Drag::WireFrom(id) if id.0 < graph.nodes.len() => {
                    let n = &graph.nodes[id.0];
                    Some(lay.output_pos(n.pos, n.kind.inputs()))
                }
                Drag::WireTo(id, port) if id.0 < graph.nodes.len() => {
                    let n = &graph.nodes[id.0];
                    Some(lay.input_pos(n.pos, n.kind.inputs(), port))
                }
                _ => None,
            };
            if let Some(a) = anchor {
                bezier(p, a, cursor, Color32::from_rgb(190, 200, 215), 1.6);
            }
        }
    }

    fn draw_nodes(&self, p: &egui::Painter, lay: &Layout, graph: &NodeGraph) {
        for (i, n) in graph.nodes.iter().enumerate() {
            let inputs = n.kind.inputs();
            let rect = lay.node_rect(n.pos, inputs);
            let accent = category_color(n.kind.category());
            let in_cycle = graph.cycle_nodes().contains(&i);

            p.rect_filled(rect, 5.0, Color32::from_rgb(38, 41, 47));
            let border = if in_cycle {
                Stroke::new(2.0, Color32::from_rgb(190, 80, 70))
            } else if self.selected == Some(NodeId(i)) {
                Stroke::new(2.0, Color32::from_rgb(225, 230, 238))
            } else {
                Stroke::new(1.0, accent)
            };
            p.rect_stroke(rect, 5.0, border, egui::StrokeKind::Outside);
            p.rect_filled(
                Rect::from_min_size(rect.min, vec2(rect.width(), TITLE_H * lay.zoom)),
                5.0,
                accent.gamma_multiply(if n.bypass { 0.25 } else { 0.55 }),
            );

            // Floor the font rather than fading it out: zooming out is how
            // you read a large patch, and a canvas of anonymous coloured
            // boxes is useless exactly when it matters most. Titles are
            // truncated to the box width instead of a fixed character
            // count, so they stay inside the node at any zoom.
            let (fs, max_chars) = title_metrics(lay.zoom, rect.width());
            p.text(
                rect.min + vec2(8.0, TITLE_H * lay.zoom * 0.5),
                Align2::LEFT_CENTER,
                truncate(&n.kind.title(), max_chars),
                FontId::proportional(fs),
                Color32::from_rgb(232, 236, 240),
            );
            // Port labels and the live readout need more room than the
            // title; below this they are dropped rather than overlapped.
            if lay.zoom >= 0.7 {
                // Input rows, labelled so a Math node's a/b are not a guess.
                for port in 0..inputs {
                    let y = rect.top() + (TITLE_H + ROW_H * (port as f32 + 0.5)) * lay.zoom;
                    p.text(
                        pos2(rect.left() + 9.0, y),
                        Align2::LEFT_CENTER,
                        n.kind.input_label(port),
                        FontId::proportional(fs * 0.82),
                        Color32::from_rgb(150, 158, 168),
                    );
                }
                // Live value, right-aligned on the first row.
                p.text(
                    pos2(rect.right() - 9.0, rect.top() + (TITLE_H + ROW_H * 0.5) * lay.zoom),
                    Align2::RIGHT_CENTER,
                    format!("{:+.2}", graph.value(NodeId(i))),
                    FontId::monospace(fs * 0.82),
                    Color32::from_rgb(190, 200, 212),
                );
            }

            // Ports last so they sit above the body, matching hit order.
            let r = PORT_R * lay.zoom.clamp(0.6, 1.4);
            p.circle_filled(
                lay.output_pos(n.pos, inputs),
                r,
                Color32::from_rgb(205, 212, 220),
            );
            for port in 0..inputs {
                let connected = graph.edges.iter().any(|e| e.to.0 == i && e.port == port);
                p.circle_filled(
                    lay.input_pos(n.pos, inputs, port),
                    r,
                    if connected {
                        Color32::from_rgb(205, 212, 220)
                    } else {
                        Color32::from_rgb(105, 112, 122)
                    },
                );
            }
        }
    }

    /// Parameters for the selected node. Editing happens here rather than
    /// inside the node box: inline widgets would have to be laid out and
    /// hit-tested through the zoom transform, and become unusable when
    /// zoomed out — exactly when a patch is big enough to need editing.
    fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        graph: &mut NodeGraph,
        registry: &ParamRegistry,
        changed: &mut bool,
    ) {
        let Some(id) = self.selected.filter(|s| s.0 < graph.nodes.len()) else {
            return;
        };
        let mut delete = false;
        egui::Frame::new()
            .fill(Color32::from_rgb(30, 33, 38))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let node = &mut graph.nodes[id.0];
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(node.kind.title()).strong());
                    if ui.checkbox(&mut node.bypass, "bypass").changed() {
                        *changed = true;
                    }
                    if ui.small_button("delete").clicked() {
                        delete = true;
                    }
                });
                node_editor(ui, &mut node.kind, registry, changed);
            });
        // Outside the closure: the node is borrowed inside it, and
        // removing it there would also leave `selected` pointing at a slot
        // that has shifted.
        if delete {
            graph.remove(id);
            self.selected = None;
            *changed = true;
        }
    }
}

/// Per-kind parameter widgets.
fn node_editor(
    ui: &mut egui::Ui,
    kind: &mut NodeKind,
    registry: &ParamRegistry,
    changed: &mut bool,
) {
    use vizz_mod::graph::{CurveShape, MathOp};
    match kind {
        NodeKind::Lfo(lfo) => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("shape")
                    .selected_text(lfo.shape.label())
                    .show_ui(ui, |ui| {
                        for s in vizz_mod::Shape::ALL {
                            ui.selectable_value(&mut lfo.shape, s, s.label());
                        }
                    });
                let mut beats = match lfo.rate {
                    vizz_mod::Rate::Beats(b) => b,
                    vizz_mod::Rate::Hz(h) => h,
                };
                let synced = matches!(lfo.rate, vizz_mod::Rate::Beats(_));
                let mut sync = synced;
                ui.checkbox(&mut sync, "sync");
                ui.add(egui::DragValue::new(&mut beats).speed(0.05).range(0.01..=64.0));
                lfo.rate = if sync {
                    vizz_mod::Rate::Beats(beats)
                } else {
                    vizz_mod::Rate::Hz(beats)
                };
            });
        }
        NodeKind::Band(i) => {
            ui.horizontal(|ui| {
                ui.label("band");
                for b in 0..4usize {
                    ui.selectable_value(i, b, format!("{}", b + 1));
                }
            });
        }
        NodeKind::Phasor { beats } => {
            ui.add(egui::DragValue::new(beats).speed(0.1).range(0.25..=64.0).suffix(" beats"));
        }
        NodeKind::Constant(c) => {
            ui.add(egui::Slider::new(c, -1.0..=1.0).text("value"));
        }
        NodeKind::Curve { shape, amount } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("curve")
                    .selected_text(shape.label())
                    .show_ui(ui, |ui| {
                        for s in CurveShape::ALL {
                            ui.selectable_value(shape, s, s.label());
                        }
                    });
                ui.add(egui::Slider::new(amount, 0.0..=1.0).text("amount"));
            });
        }
        NodeKind::Math { op } => {
            egui::ComboBox::from_id_salt("math")
                .selected_text(op.label())
                .show_ui(ui, |ui| {
                    for o in MathOp::ALL {
                        ui.selectable_value(op, o, o.label());
                    }
                });
        }
        NodeKind::Scale { mul, add } => {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(mul).speed(0.01).prefix("× "));
                ui.add(egui::DragValue::new(add).speed(0.01).prefix("+ "));
            });
        }
        NodeKind::Smooth { attack, release } => {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(attack).speed(0.005).range(0.0..=2.0).prefix("A "));
                ui.add(egui::DragValue::new(release).speed(0.005).range(0.0..=5.0).prefix("R "));
            });
        }
        NodeKind::Quantise { steps } => {
            ui.add(egui::Slider::new(steps, 1.0..=32.0).text("steps"));
        }
        NodeKind::Param { addr, depth } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("param")
                    .selected_text(if addr.is_empty() { "— pick —" } else { addr.as_str() })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (_, def) in registry.iter() {
                            if ui
                                .selectable_label(addr.as_str() == def.addr, &def.addr)
                                .clicked()
                            {
                                *addr = def.addr.clone();
                                *changed = true;
                            }
                        }
                    });
                ui.add(egui::Slider::new(depth, -1.0..=1.0).text("depth"));
            });
        }
        NodeKind::Level | NodeKind::SampleHold => {
            ui.small("no parameters");
        }
    }
}

fn group_label(c: Category) -> &'static str {
    match c {
        Category::Source => "Sources",
        Category::Operator => "Operators",
        Category::Sink => "Outputs",
    }
}

fn category_color(c: Category) -> Color32 {
    match c {
        Category::Source => Color32::from_rgb(70, 120, 175),
        Category::Operator => Color32::from_rgb(150, 120, 60),
        Category::Sink => Color32::from_rgb(70, 140, 100),
    }
}

/// Title font size and how many characters fit, for a node of `width`.
///
/// Extracted because this is where the legibility bug lived: the font
/// clamp floor sat below the threshold that decided whether to draw at
/// all, so titles silently vanished below ~0.63 zoom — precisely the zoom
/// you use to read a large patch.
fn title_metrics(zoom: f32, width: f32) -> (f32, usize) {
    let fs = (12.0 * zoom).clamp(8.0, 18.0);
    let chars = ((width - 16.0) / (fs * 0.52)).floor().max(3.0) as usize;
    (fs, chars)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// Horizontal-tangent cubic — the wire shape every patcher uses, and the
/// reason is practical: it leaves ports horizontally so a wire never
/// ambiguously overlaps the node it came from.
fn bezier(p: &egui::Painter, a: Pos2, b: Pos2, color: Color32, width: f32) {
    let dx = ((b.x - a.x).abs() * 0.5).max(30.0);
    p.add(egui::Shape::CubicBezier(
        egui::epaint::CubicBezierShape::from_points_stroke(
            [a, pos2(a.x + dx, a.y), pos2(b.x - dx, b.y), b],
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        ),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizz_mod::graph::NodeKind;

    fn lay(zoom: f32) -> Layout {
        Layout { origin: pos2(100.0, 50.0), pan: vec2(10.0, 20.0), zoom }
    }

    /// The transform must round-trip at any zoom, or a node lands somewhere
    /// other than where it was dropped — which reads as the canvas
    /// "drifting" and is maddening to use.
    #[test]
    fn screen_and_graph_space_round_trip() {
        for zoom in [0.35, 0.5, 1.0, 1.7, 2.5] {
            let l = lay(zoom);
            for p in [[0.0, 0.0], [120.0, -60.0], [-45.5, 900.0]] {
                let back = l.to_graph(l.to_screen(p));
                assert!(
                    (back[0] - p[0]).abs() < 1e-3 && (back[1] - p[1]).abs() < 1e-3,
                    "zoom {zoom}: {p:?} -> {back:?}"
                );
            }
        }
    }

    /// Ports must sit on their node's edge at every zoom. If they drift
    /// outside, a wire appears to start in empty space; if inside, the port
    /// is unreachable because the body swallows the press.
    #[test]
    fn ports_sit_on_the_node_edge() {
        for zoom in [0.35, 1.0, 2.5] {
            let l = lay(zoom);
            let pos = [30.0, 40.0];
            let rect = l.node_rect(pos, 2);
            let out = l.output_pos(pos, 2);
            assert!((out.x - rect.right()).abs() < 1e-3, "output off the right edge");
            assert!(rect.y_range().contains(out.y), "output outside vertically");
            for port in 0..2 {
                let inp = l.input_pos(pos, 2, port);
                assert!((inp.x - rect.left()).abs() < 1e-3, "input off the left edge");
                assert!(rect.y_range().contains(inp.y), "input {port} outside vertically");
            }
            // Two inputs must not land on the same point, or one is
            // unreachable.
            assert!(
                l.input_pos(pos, 2, 0).distance(l.input_pos(pos, 2, 1)) > 4.0,
                "input ports collapsed together at zoom {zoom}"
            );
        }
    }

    /// Ports win over bodies, and later nodes win over earlier ones. Get
    /// either wrong and a wire drag silently becomes a node move.
    #[test]
    fn hit_testing_prefers_ports_then_topmost_node() {
        let l = lay(1.0);
        let mut g = NodeGraph::default();
        let a = g.add(NodeKind::Constant(1.0), [0.0, 0.0]);
        // Overlapping, added later, so it draws on top.
        let b = g.add(NodeKind::Scale { mul: 1.0, add: 0.0 }, [20.0, 10.0]);

        let b_out = l.output_pos([20.0, 10.0], 1);
        assert_eq!(l.port_at(&g, b_out), Some((b, None)), "output port missed");
        let b_in = l.input_pos([20.0, 10.0], 1, 0);
        assert_eq!(l.port_at(&g, b_in), Some((b, Some(0))), "input port missed");

        // A point inside both bodies belongs to the topmost.
        let overlap = l.node_rect([20.0, 10.0], 1).center();
        assert_eq!(l.node_at(&g, overlap), Some(b), "grabbed the node underneath");

        // A point inside only the lower node still finds it.
        let just_a = l.to_screen([4.0, 4.0]);
        assert_eq!(l.node_at(&g, just_a), Some(a));

        // Empty canvas hits nothing, so a drag there pans.
        assert_eq!(l.node_at(&g, l.to_screen([900.0, 900.0])), None);
        assert_eq!(l.port_at(&g, l.to_screen([900.0, 900.0])), None);
    }

    /// Zooming keeps the point under the cursor fixed — the behaviour that
    /// makes a canvas feel anchored rather than sliding away.
    #[test]
    fn zoom_about_cursor_keeps_the_point_under_it() {
        let origin = pos2(0.0, 0.0);
        let cursor = pos2(300.0, 200.0);
        let mut view = GraphView::default();

        let before = view.layout(origin).to_graph(cursor);
        // Same arithmetic the scroll handler runs.
        let old = view.layout(origin);
        view.zoom = (view.zoom * 1.4).clamp(0.35, 2.5);
        let after = view.layout(origin).to_graph(cursor);
        view.pan += vec2(after[0] - before[0], after[1] - before[1]);
        let _ = old;

        let settled = view.layout(origin).to_graph(cursor);
        assert!(
            (settled[0] - before[0]).abs() < 1e-3 && (settled[1] - before[1]).abs() < 1e-3,
            "cursor drifted: {before:?} -> {settled:?}"
        );
    }

    /// Titles must stay legible at every zoom the canvas allows, and must
    /// stay inside their box. The bug this guards was silent: below ~0.63
    /// zoom nodes rendered as anonymous coloured rectangles, which is
    /// exactly the zoom you use to read a patch too big to fit.
    #[test]
    fn titles_stay_legible_and_inside_the_box_at_every_zoom() {
        for zoom in [0.35f32, 0.55, 0.7, 1.0, 2.5] {
            let width = NODE_W * zoom;
            let (fs, chars) = title_metrics(zoom, width);
            assert!(fs >= 8.0, "font {fs} at zoom {zoom} is too small to read");
            assert!(chars >= 3, "only {chars} characters fit at zoom {zoom}");
            // Truncated text must fit the box at the size it will be drawn.
            let title = "/particles/saturation";
            let drawn = truncate(title, chars);
            let est = drawn.chars().count() as f32 * fs * 0.52;
            assert!(
                est <= width - 12.0,
                "title overflows at zoom {zoom}: {est:.1}px in {width:.1}px box"
            );
        }
    }

    #[test]
    fn truncate_never_exceeds_its_budget() {
        assert_eq!(truncate("short", 20), "short");
        for max in 3..12 {
            let out = truncate("/particles/saturation", max);
            assert!(out.chars().count() <= max, "{out:?} exceeds {max}");
        }
    }

    /// Node height has to grow with input count, or a two-input node's
    /// second port falls outside its own box.
    #[test]
    fn node_height_follows_input_count() {
        let l = lay(1.0);
        let one = l.node_rect([0.0, 0.0], 1).height();
        let two = l.node_rect([0.0, 0.0], 2).height();
        assert!(two > one, "two-input node is not taller: {one} vs {two}");
        assert!(l.node_rect([0.0, 0.0], 2).y_range().contains(l.input_pos([0.0, 0.0], 2, 1).y));
    }
}
