//! The options of the active tool: what the Properties panel's Tool section and the toolbox's Options area show in
//! Animate (object drawing, snapping, pencil mode, brush and eraser settings, bucket gap size, rectangle and oval
//! settings).

use anicraft_tools::{BrushMode, EraserMode, GapSize, LassoMode, PaintBrushStyle, PencilMode, PolyStarStyle, TipShape, ToolId, ToolSettings};
use anim_scene::TextKind;
use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use super::ToolUiEvent;

fn patch(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Tools whose drawing is placed by the snapping switches.
const fn snaps(tool: ToolId) -> bool {
    matches!(
        tool,
        ToolId::Selection
            | ToolId::Subselection
            | ToolId::FreeTransform
            | ToolId::Line
            | ToolId::Rectangle
            | ToolId::Oval
            | ToolId::RectanglePrimitive
            | ToolId::OvalPrimitive
            | ToolId::PolyStar
            | ToolId::Pen
            | ToolId::Pencil
            | ToolId::Brush
    )
}

const fn draws_objects(tool: ToolId) -> bool {
    matches!(tool, ToolId::Line | ToolId::Rectangle | ToolId::Oval | ToolId::PolyStar | ToolId::Pen | ToolId::Pencil | ToolId::Brush)
}

const fn text_kind_label(kind: TextKind) -> &'static str {
    match kind {
        TextKind::Static => "Static Text",
        TextKind::Dynamic => "Dynamic Text",
        TextKind::Input => "Input Text",
    }
}

fn combo<T: Copy + PartialEq>(ui: &mut Ui, id: &str, current: T, all: &[T], label: impl Fn(T) -> &'static str) -> Option<T> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id).selected_text(label(current)).show_ui(ui, |ui| {
        for &item in all {
            if ui.selectable_label(item == current, label(item)).clicked() && item != current {
                chosen = Some(item);
            }
        }
    });
    chosen
}

/// A number field. Returns the new value while it is being edited.
fn number(ui: &mut Ui, value: f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> Option<f64> {
    let mut v = value;
    let r = ui.add(egui::DragValue::new(&mut v).range(range).speed(0.25).suffix(suffix));
    r.changed().then_some(v)
}

const fn pencil_label(m: PencilMode) -> &'static str {
    match m {
        PencilMode::Straighten => "Straighten",
        PencilMode::Smooth => "Smooth",
        PencilMode::Ink => "Ink",
    }
}

const fn eraser_label(m: EraserMode) -> &'static str {
    match m {
        EraserMode::Normal => "Erase Normal",
        EraserMode::Fills => "Erase Fills",
        EraserMode::Lines => "Erase Lines",
        EraserMode::SelectedFills => "Erase Selected Fills",
        EraserMode::Inside => "Erase Inside",
    }
}

const fn gap_label(g: GapSize) -> &'static str {
    match g {
        GapSize::None => "Don't close gaps",
        GapSize::Small => "Close small gaps",
        GapSize::Medium => "Close medium gaps",
        GapSize::Large => "Close large gaps",
    }
}

/// Draws the options of `tool`; returns the `tools.options` patches the person made.
pub fn tool_options(ui: &mut Ui, tool: ToolId, s: &ToolSettings) -> Vec<ToolUiEvent> {
    let mut events = Vec::new();
    let mut set = |value: Value| events.push(ToolUiEvent::options(value));
    ui.label(egui::RichText::new(tool.label()).strong());
    ui.weak(tool.description());
    ui.add_space(4.0);
    egui::Grid::new(("tool-options", tool.toolbar_id())).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        if draws_objects(tool) {
            ui.label("Drawing");
            let mut on = s.object_drawing;
            if ui.checkbox(&mut on, "Object drawing").changed() {
                set(json!({ "object_drawing": on }));
            }
            ui.end_row();
        }
        match tool {
            ToolId::Rectangle | ToolId::RectanglePrimitive => {
                ui.label("Corner radius");
                ui.vertical(|ui| {
                    let mut lock = s.corner_lock;
                    if ui.checkbox(&mut lock, "Same radius for all corners").changed() {
                        set(json!({ "corner_lock": lock }));
                    }
                    if s.corner_lock {
                        if let Some(v) = number(ui, s.corner_radius, 0.0..=1000.0, " px") {
                            set(json!({ "corner_radius": v }));
                        }
                    } else {
                        let names = ["Top left", "Top right", "Bottom right", "Bottom left"];
                        for (i, (name, radius)) in names.into_iter().zip(s.corner_radii).enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(name);
                                if let Some(v) = number(ui, radius, 0.0..=1000.0, " px") {
                                    let mut radii = s.corner_radii;
                                    if let Some(slot) = radii.get_mut(i) {
                                        *slot = v;
                                    }
                                    set(json!({ "corner_radii": radii }));
                                }
                            });
                        }
                    }
                });
                ui.end_row();
            }
            ToolId::PolyStar => {
                ui.label("Style");
                if let Some(style) = combo(ui, "polystar-style", s.polystar.style, &PolyStarStyle::ALL, PolyStarStyle::label) {
                    set(json!({ "polystar": { "style": patch(style) } }));
                }
                ui.end_row();
                ui.label("Number of sides");
                let mut sides = s.polystar.sides;
                if ui.add(egui::DragValue::new(&mut sides).range(3..=32)).changed() {
                    set(json!({ "polystar": { "sides": sides } }));
                }
                ui.end_row();
                ui.label("Star point size");
                let mut size = s.polystar.star_point_size;
                let enabled = s.polystar.style == PolyStarStyle::Star;
                if ui.add_enabled(enabled, egui::DragValue::new(&mut size).range(0.0..=1.0).speed(0.01).fixed_decimals(2)).changed() {
                    set(json!({ "polystar": { "star_point_size": size } }));
                }
                ui.end_row();
            }
            ToolId::PaintBrush => {
                ui.label("Brush");
                if let Some(style) = combo(ui, "paintbrush-style", s.paint_brush.style, &PaintBrushStyle::ALL, PaintBrushStyle::label) {
                    set(json!({ "paint_brush": { "style": patch(style) } }));
                }
                ui.end_row();
                ui.label("Size");
                if let Some(v) = number(ui, s.paint_brush.size, 0.5..=500.0, " px") {
                    set(json!({ "paint_brush": { "size": v } }));
                }
                ui.end_row();
                if s.paint_brush.style.is_pattern() {
                    ui.label("Spacing");
                    if let Some(v) = number(ui, s.paint_brush.spacing, 10.0..=1000.0, " %") {
                        set(json!({ "paint_brush": { "spacing": v } }));
                    }
                    ui.end_row();
                }
                ui.label("Smoothing");
                let mut smoothing = s.paint_brush.smoothing;
                if ui.add(egui::Slider::new(&mut smoothing, 0.0..=100.0).fixed_decimals(0)).changed() {
                    set(json!({ "paint_brush": { "smoothing": smoothing } }));
                }
                ui.end_row();
                ui.label("");
                ui.weak("Paints with the stroke colour.");
                ui.end_row();
            }
            ToolId::Width => {
                ui.label("");
                ui.weak("Select a shape, drag out from its stroke to widen it there. The Properties panel offers ready-made width profiles.");
                ui.end_row();
            }
            ToolId::Lasso => {
                ui.label("Mode");
                if let Some(mode) = combo(ui, "lasso-mode", s.lasso.mode, &LassoMode::ALL, LassoMode::label) {
                    set(json!({ "lasso": { "mode": patch(mode) } }));
                }
                ui.end_row();
                if s.lasso.mode == LassoMode::MagicWand {
                    ui.label("Threshold");
                    if let Some(v) = number(ui, s.lasso.threshold, 0.0..=255.0, "") {
                        set(json!({ "lasso": { "threshold": v } }));
                    }
                    ui.end_row();
                }
                ui.label("");
                ui.weak("Hold Alt and click to place polygon corners; Enter closes the polygon.");
                ui.end_row();
            }
            ToolId::Text => {
                ui.label("Text type");
                let kinds = [TextKind::Static, TextKind::Dynamic, TextKind::Input];
                if let Some(kind) = combo(ui, "text-kind", s.text.kind, &kinds, text_kind_label) {
                    set(json!({ "text": { "kind": patch(kind) } }));
                }
                ui.end_row();
                ui.label("");
                ui.weak("Click for a text field that grows as you type; drag for a box of a fixed width.");
                ui.end_row();
            }
            ToolId::Oval | ToolId::OvalPrimitive => {
                ui.label("Start angle");
                if let Some(v) = number(ui, s.oval.start_angle, 0.0..=360.0, "\u{b0}") {
                    set(json!({ "oval": { "start_angle": v } }));
                }
                ui.end_row();
                ui.label("End angle");
                if let Some(v) = number(ui, s.oval.end_angle, 0.0..=360.0, "\u{b0}") {
                    set(json!({ "oval": { "end_angle": v } }));
                }
                ui.end_row();
                ui.label("Inner radius");
                if let Some(v) = number(ui, s.oval.inner_radius, 0.0..=99.0, "%") {
                    set(json!({ "oval": { "inner_radius": v } }));
                }
                ui.end_row();
                ui.label("");
                let mut close = s.oval.close_path;
                if ui.checkbox(&mut close, "Close path").changed() {
                    set(json!({ "oval": { "close_path": close } }));
                }
                ui.end_row();
                ui.label("");
                if ui.button("Reset oval").clicked() {
                    set(json!({ "oval": { "start_angle": 0.0, "end_angle": 0.0, "inner_radius": 0.0, "close_path": true } }));
                }
                ui.end_row();
            }
            ToolId::Pencil => {
                ui.label("Mode");
                if let Some(m) = combo(ui, "pencil-mode", s.pencil_mode, &[PencilMode::Straighten, PencilMode::Smooth, PencilMode::Ink], pencil_label) {
                    set(json!({ "pencil_mode": patch(m) }));
                }
                ui.end_row();
            }
            ToolId::Brush => {
                ui.label("Size");
                if let Some(v) = number(ui, s.brush_size, 0.5..=500.0, " px") {
                    set(json!({ "brush_size": v }));
                }
                ui.end_row();
                ui.label("Shape");
                if let Some(m) = combo(ui, "brush-shape", s.brush_shape, &TipShape::ALL, TipShape::label) {
                    set(json!({ "brush_shape": patch(m) }));
                }
                ui.end_row();
                ui.label("Mode");
                if let Some(m) = combo(ui, "brush-mode", s.brush_mode, &BrushMode::ALL, BrushMode::label) {
                    set(json!({ "brush_mode": patch(m) }));
                }
                ui.end_row();
                ui.label("Smoothing");
                let mut smoothing = s.brush_smoothing;
                if ui.add(egui::Slider::new(&mut smoothing, 0.0..=100.0).fixed_decimals(0)).changed() {
                    set(json!({ "brush_smoothing": smoothing }));
                }
                ui.end_row();
                ui.label("Pressure");
                let mut pressure = s.brush_pressure;
                if ui.checkbox(&mut pressure, "Use pen pressure").changed() {
                    set(json!({ "brush_pressure": pressure }));
                }
                ui.end_row();
            }
            ToolId::PaintBucket => {
                ui.label("Gap size");
                if let Some(g) = combo(ui, "bucket-gap", s.bucket_gap, &[GapSize::None, GapSize::Small, GapSize::Medium, GapSize::Large], gap_label) {
                    set(json!({ "bucket_gap": patch(g) }));
                }
                ui.end_row();
                ui.label("Lock fill");
                let mut lock = s.bucket_lock_fill;
                if ui.checkbox(&mut lock, "Lock gradient and bitmap fills").changed() {
                    set(json!({ "bucket_lock_fill": lock }));
                }
                ui.end_row();
            }
            ToolId::Eraser => {
                ui.label("Mode");
                let modes = [EraserMode::Normal, EraserMode::Fills, EraserMode::Lines, EraserMode::SelectedFills, EraserMode::Inside];
                if let Some(m) = combo(ui, "eraser-mode", s.eraser_mode, &modes, eraser_label) {
                    set(json!({ "eraser_mode": patch(m) }));
                }
                ui.end_row();
                ui.label("Shape");
                if let Some(m) = combo(ui, "eraser-shape", s.eraser_shape, &TipShape::ALL, TipShape::label) {
                    set(json!({ "eraser_shape": patch(m) }));
                }
                ui.end_row();
                ui.label("Size");
                if let Some(v) = number(ui, s.eraser_size, 1.0..=500.0, " px") {
                    set(json!({ "eraser_size": v }));
                }
                ui.end_row();
                ui.label("Faucet");
                let mut faucet = s.eraser_faucet;
                if ui.checkbox(&mut faucet, "Click to erase a whole fill or stroke").changed() {
                    set(json!({ "eraser_faucet": faucet }));
                }
                ui.end_row();
            }
            ToolId::Rotation3D | ToolId::Translation3D => {
                ui.label("");
                ui.weak("Select a movie clip first. Set the perspective angle and vanishing point in the document properties.");
                ui.end_row();
            }
            ToolId::GradientTransform => {
                ui.label("");
                ui.weak("Click a gradient or bitmap fill, then drag its handles.");
                ui.end_row();
            }
            _ => {}
        }
        if snaps(tool) {
            ui.label("Snapping");
            ui.vertical(|ui| {
                let mut objects = s.snap.objects;
                if ui.checkbox(&mut objects, "Snap to objects").changed() {
                    set(json!({ "snap": { "objects": objects } }));
                }
                let mut pixels = s.snap.pixels;
                if ui.checkbox(&mut pixels, "Snap to pixels").changed() {
                    set(json!({ "snap": { "pixels": pixels } }));
                }
                let mut grid = s.snap.grid;
                if ui.checkbox(&mut grid, "Snap to grid").changed() {
                    set(json!({ "snap": { "grid": grid } }));
                }
                if s.snap.grid {
                    ui.horizontal(|ui| {
                        ui.label("Grid size");
                        if let Some(v) = number(ui, s.snap.grid_size, 1.0..=500.0, " px") {
                            set(json!({ "snap": { "grid_size": v } }));
                        }
                    });
                }
                let mut guides = s.snap.guides;
                if ui.checkbox(&mut guides, "Snap to guides").changed() {
                    set(json!({ "snap": { "guides": guides } }));
                }
            });
            ui.end_row();
        }
    });
    events
}
