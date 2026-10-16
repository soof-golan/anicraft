//! The Motion Editor window, the custom ease editor, and the motion path and shape hint handles on the stage.
//!
//! Everything here is a pure function of the document and the editor state: [`tween_view`] reads what the keyframe
//! under the playhead tweens, [`editor`] draws it and answers with commands (`(id, params)` pairs the app runs, so
//! every change is an undo step and an MCP tool), [`handles`] and [`overlays`] describe what is drawn on the stage
//! and [`pick`] what a pointer press grabs.

#![allow(clippy::suboptimal_flops, reason = "screen geometry reads better as plain arithmetic")]

use anicraft_cli::mcp::AniDoc;
use anim_core::num;
use anim_scene::motion::{self, MOTION_PRESETS, MotionProp};
use anim_scene::timeline::{EaseCurve, EaseTarget, ShapeHint, TweenType};
use anim_timeline::motion::MotionTracks;
use anim_ui::stage::{MarkerShape, Overlay, OverlayStyle};
use egui::{Color32, Pos2, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

/// A command to run: its id and parameters.
pub type Cmd = (&'static str, Value);

/// What the keyframe under the playhead tweens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    None,
    Classic,
    Shape,
    Motion,
}

/// One property keyframe as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyView {
    /// Frames after the span start.
    pub offset: u32,
    pub value: f64,
    pub ease: i32,
    pub curve: bool,
}

/// One property track as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackView {
    pub prop: MotionProp,
    pub keys: Vec<KeyView>,
    /// The value at the playhead.
    pub now: f64,
    /// The value at every frame of the span.
    pub samples: Vec<f64>,
}

/// The tween under the playhead.
#[derive(Debug, Clone, PartialEq)]
pub struct TweenView {
    pub layer: usize,
    /// Playhead, 1-based as commands take it.
    pub frame: u32,
    /// First frame of the span, 0-based.
    pub start: u32,
    pub span: u32,
    /// Frames from the span start to the playhead.
    pub offset: u32,
    pub kind: Kind,
    pub tracks: Vec<TrackView>,
    pub acceleration: i32,
    pub custom: Option<Vec<[f64; 2]>>,
    pub hints: Vec<ShapeHint>,
}

/// Which end of a thing on the stage is grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// A motion path point `offset` frames into the span.
    Path(u32),
    HintStart(u32),
    /// The end point of hint `hint` of the shape tween that starts at the 1-based frame `tween`.
    HintEnd {
        tween: u32,
        hint: u32,
    },
}

/// A handle being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drag {
    pub handle: Handle,
    pub pos: Pos2,
}

/// The ease curve being edited, with its two inner control points.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveEdit {
    pub target: CurveTarget,
    pub p1: [f64; 2],
    pub p2: [f64; 2],
}

/// What a [`CurveEdit`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveTarget {
    Key(MotionProp, u32),
    Tween(EaseTarget),
}

/// Transient state of the Motion Editor.
#[derive(Debug, Clone, Default)]
pub struct MotionUi {
    pub open: bool,
    pub selected: Option<(MotionProp, u32)>,
    pub curve: Option<CurveEdit>,
    pub drag: Option<Drag>,
    pub preset: usize,
}

/// The stage values of `prop` for the first object of a keyframe.
fn base_value(keyframe: &anim_scene::Frame, prop: MotionProp) -> f64 {
    keyframe.elements.first().map_or(0.0, |e| {
        let alpha = e.as_symbol_instance().map_or(100.0, |i| f64::from(i.color.mul[3]) * 100.0);
        prop.of_matrix(e.matrix().unwrap_or(anim_core::Affine::IDENTITY), alpha)
    })
}

/// Reads the tween the keyframe under the playhead starts (or sits inside), for the Motion Editor.
#[must_use]
pub fn tween_view(doc: &AniDoc) -> Option<TweenView> {
    let state = &doc.editor.state;
    let layer = doc.document().timeline(&state.timeline)?.layers.get(state.layer)?;
    let keyframe = layer.keyframe_at(state.frame)?;
    let kind = match keyframe.tween_type {
        TweenType::None => Kind::None,
        TweenType::Classic => Kind::Classic,
        TweenType::Shape => Kind::Shape,
        TweenType::MotionObject => Kind::Motion,
    };
    let steps = keyframe.duration.saturating_sub(1);
    let tracks = motion::motion_object(keyframe)
        .map(|object| {
            let eval = MotionTracks::from_object(object.clone());
            object
                .tracks
                .iter()
                .map(|track| {
                    let base = base_value(keyframe, track.prop);
                    let first = track.keys.first().map_or(base, |k| k.value);
                    let at = |t: f64| base + (eval.value_at(track.prop, t).unwrap_or(first) - first);
                    let t_of = |offset: u32| if steps == 0 { 0.0 } else { f64::from(offset) / f64::from(steps) };
                    let offset_now = state.frame.saturating_sub(keyframe.index).min(steps);
                    TrackView {
                        prop: track.prop,
                        keys: track
                            .keys
                            .iter()
                            .map(|k| KeyView {
                                offset: num::round_f64_to_u32(k.t * f64::from(steps)),
                                value: base + (k.value - first),
                                ease: k.ease,
                                curve: k.curve.is_some(),
                            })
                            .collect(),
                        now: at(t_of(offset_now)),
                        samples: (0..=steps).map(|o| at(t_of(o))).collect(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(TweenView {
        layer: state.layer,
        frame: state.frame.saturating_add(1),
        start: keyframe.index,
        span: keyframe.duration,
        offset: state.frame.saturating_sub(keyframe.index),
        kind,
        tracks,
        acceleration: keyframe.acceleration,
        custom: keyframe.eases.iter().find_map(|e| match (&e.curve, e.target) {
            (EaseCurve::Custom(points), EaseTarget::All) => Some(points.clone()),
            _ => None,
        }),
        hints: keyframe.shape_hints.clone(),
    })
}

fn params(view: &TweenView, extra: Value) -> Value {
    let mut base = json!({ "layer": view.layer, "frame": view.frame });
    if let (Some(map), Some(more)) = (base.as_object_mut(), extra.as_object()) {
        map.extend(more.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    base
}

/// The 1-based frame the commands address: the first frame of the span.
fn span_params(view: &TweenView, extra: Value) -> Value {
    let mut p = params(view, extra);
    if let Some(map) = p.as_object_mut() {
        map.insert("frame".into(), json!(view.start.saturating_add(1)));
    }
    p
}

fn property_label(prop: MotionProp) -> &'static str {
    match prop {
        MotionProp::X => "X",
        MotionProp::Y => "Y",
        MotionProp::Rotation => "Rotation",
        MotionProp::ScaleX => "Scale X %",
        MotionProp::ScaleY => "Scale Y %",
        MotionProp::Alpha => "Alpha %",
    }
}

/// The curve of an editor: the cubic bezier through (0,0), p1, p2, (1,1).
fn curve_points(edit: &CurveEdit) -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], edit.p1, edit.p2, [1.0, 1.0]]
}

fn bezier_at(edit: &CurveEdit, u: f64) -> [f64; 2] {
    let w = 1.0 - u;
    let [p0, p1, p2, p3] = [[0.0, 0.0], edit.p1, edit.p2, [1.0, 1.0]];
    let axis = |i: usize| {
        let pick = |p: [f64; 2]| p.get(i).copied().unwrap_or(0.0);
        (w.powi(3) * pick(p0)) + (3.0 * w * w * u * pick(p1)) + (3.0 * w * u * u * pick(p2)) + (u.powi(3) * pick(p3))
    };
    [axis(0), axis(1)]
}

/// The bezier ease editor: a square with the curve and two draggable control points. Returns true when a point moved.
pub fn bezier_editor(ui: &mut Ui, edit: &mut CurveEdit) -> bool {
    let side = 200.0;
    let (response, painter) = ui.allocate_painter(vec2(side, side), Sense::hover());
    let rect = response.rect.shrink(8.0);
    let to_screen =
        |p: [f64; 2]| pos2(rect.left() + num::f64_to_f32(p[0]) * rect.width(), rect.bottom() - num::f64_to_f32((p[1] + 0.25) / 1.5) * rect.height());
    painter.rect_filled(response.rect, 3.0, ui.visuals().extreme_bg_color);
    let weak = ui.visuals().weak_text_color();
    painter.line_segment([to_screen([0.0, 0.0]), to_screen([1.0, 1.0])], Stroke::new(1.0, weak.gamma_multiply(0.5)));
    painter.line_segment([to_screen([0.0, 0.0]), to_screen(edit.p1)], Stroke::new(1.0, weak));
    painter.line_segment([to_screen([1.0, 1.0]), to_screen(edit.p2)], Stroke::new(1.0, weak));
    let line: Vec<Pos2> = (0..=48).map(|i| to_screen(bezier_at(edit, f64::from(i) / 48.0))).collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, ui.visuals().selection.bg_fill)));
    let mut changed = false;
    for (index, point) in [&mut edit.p1, &mut edit.p2].into_iter().enumerate() {
        let centre = to_screen(*point);
        let hit = egui::Rect::from_center_size(centre, vec2(14.0, 14.0));
        let r = ui.interact(hit, response.id.with(("ease-point", index)), Sense::drag());
        painter.circle_filled(centre, 5.0, if r.dragged() { Color32::WHITE } else { ui.visuals().selection.bg_fill });
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            let x = f64::from((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let y = (f64::from((rect.bottom() - p.y) / rect.height()) * 1.5 - 0.25).clamp(-0.25, 1.25);
            *point = [x, y];
            changed = true;
        }
    }
    changed
}

/// A mini graph of one track across the span; returns the offset of a clicked key.
fn track_graph(ui: &mut Ui, track: &TrackView, span: u32, now: u32, selected: Option<u32>) -> Option<u32> {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width().clamp(120.0, 320.0), 44.0), Sense::click());
    let rect = response.rect;
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let steps = span.saturating_sub(1).max(1);
    let (lo, hi) = track.samples.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let range = if (hi - lo).abs() < 1e-9 { 1.0 } else { hi - lo };
    let at = |offset: u32, value: f64| {
        let x = rect.left() + 4.0 + (rect.width() - 8.0) * num::f64_to_f32(f64::from(offset) / f64::from(steps));
        let y = rect.bottom() - 6.0 - (rect.height() - 12.0) * num::f64_to_f32((value - lo) / range);
        pos2(x, y)
    };
    let line: Vec<Pos2> = track.samples.iter().enumerate().map(|(i, v)| at(num::usize_to_u32_sat(i), *v)).collect();
    if line.len() > 1 {
        painter.add(egui::Shape::line(line, Stroke::new(1.5, ui.visuals().text_color())));
    }
    let cursor = at(now.min(steps), lo).x;
    painter.line_segment([pos2(cursor, rect.top()), pos2(cursor, rect.bottom())], Stroke::new(1.0, ui.visuals().warn_fg_color));
    let mut clicked = None;
    for key in &track.keys {
        let p = at(key.offset, key.value);
        let size = if selected == Some(key.offset) { 5.0 } else { 3.5 };
        let colour = if key.ease != 0 || key.curve { ui.visuals().selection.bg_fill } else { ui.visuals().text_color() };
        painter.add(egui::Shape::convex_polygon(
            vec![pos2(p.x, p.y - size), pos2(p.x + size, p.y), pos2(p.x, p.y + size), pos2(p.x - size, p.y)],
            colour,
            Stroke::NONE,
        ));
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && pointer.distance(p) < 9.0
        {
            clicked = Some(key.offset);
        }
    }
    clicked
}

/// The window's contents. Returns the commands to run.
pub fn editor(ui: &mut Ui, view: Option<&TweenView>, state: &mut MotionUi) -> Vec<Cmd> {
    let mut out: Vec<Cmd> = Vec::new();
    let Some(view) = view else {
        ui.label("Select a frame with a symbol, group or text on a layer.");
        return out;
    };
    ui.label(format!(
        "Layer {} - frames {} to {} - {}",
        view.layer,
        view.start.saturating_add(1),
        view.start.saturating_add(view.span),
        match view.kind {
            Kind::None => "no tween",
            Kind::Classic => "classic tween",
            Kind::Shape => "shape tween",
            Kind::Motion => "motion tween",
        }
    ));
    match view.kind {
        Kind::None => {
            ui.horizontal(|ui| {
                if ui.button("Create Motion Tween").clicked() {
                    out.push(("timeline.create_motion_tween", span_params(view, json!({}))));
                }
                if ui.button("Classic Tween").clicked() {
                    out.push(("timeline.create_classic_tween", span_params(view, json!({}))));
                }
                if ui.button("Shape Tween").clicked() {
                    out.push(("timeline.create_shape_tween", span_params(view, json!({}))));
                }
            });
        }
        Kind::Motion => motion_section(ui, view, state, &mut out),
        Kind::Classic | Kind::Shape => ease_section(ui, view, state, &mut out),
    }
    if view.kind != Kind::None {
        ui.separator();
        if ui.button("Remove Tween").clicked() {
            out.push(("timeline.remove_tween", span_params(view, json!({}))));
        }
    }
    out
}

fn motion_section(ui: &mut Ui, view: &TweenView, state: &mut MotionUi, out: &mut Vec<Cmd>) {
    let selected = state.selected;
    egui::Grid::new("motion-tracks").num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
        for track in &view.tracks {
            ui.label(property_label(track.prop));
            let mut value = track.now;
            let has_key = track.keys.iter().any(|k| k.offset == view.offset);
            let editable = view.offset > 0;
            if ui
                .add_enabled(editable, egui::DragValue::new(&mut value).speed(0.5).max_decimals(2))
                .on_disabled_hover_text("The first frame is the object itself; move the object")
                .changed()
            {
                out.push(("motion.set_key", span_params(view, json!({ "property": track.prop.label(), "offset": view.offset, "value": value }))));
            }
            let label = if has_key { "Remove key" } else { "Add key" };
            if ui.add_enabled(editable, egui::Button::new(label)).clicked() {
                if has_key {
                    out.push(("motion.remove_key", span_params(view, json!({ "property": track.prop.label(), "offset": view.offset }))));
                } else {
                    out.push(("motion.set_key", span_params(view, json!({ "property": track.prop.label(), "offset": view.offset, "value": track.now }))));
                }
            }
            let sel = selected.and_then(|(p, o)| (p == track.prop).then_some(o));
            if let Some(offset) = track_graph(ui, track, view.span, view.offset, sel) {
                state.selected = Some((track.prop, offset));
            }
            ui.end_row();
        }
    });
    if let Some((prop, offset)) = state.selected
        && let Some(track) = view.tracks.iter().find(|t| t.prop == prop)
        && let Some(key) = track.keys.iter().find(|k| k.offset == offset)
    {
        ui.separator();
        ui.label(format!("{} keyframe at frame {}", property_label(prop), view.start.saturating_add(offset).saturating_add(1)));
        let mut ease = key.ease;
        ui.horizontal(|ui| {
            ui.label("Ease");
            if ui.add(egui::Slider::new(&mut ease, -100..=100)).changed() {
                out.push(("motion.set_key_ease", span_params(view, json!({ "property": prop.label(), "offset": offset, "acceleration": ease }))));
            }
            if ui.button("Custom...").clicked() {
                state.curve = Some(CurveEdit { target: CurveTarget::Key(prop, offset), p1: [0.3, 0.0], p2: [0.7, 1.0] });
            }
        });
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Preset");
        let current = MOTION_PRESETS.get(state.preset).map_or("", |p| p.name);
        egui::ComboBox::from_id_salt("motion-presets").selected_text(current).show_ui(ui, |ui| {
            for (i, p) in MOTION_PRESETS.iter().enumerate() {
                ui.selectable_value(&mut state.preset, i, p.name).on_hover_text(p.description);
            }
        });
        if ui.button("Apply").clicked()
            && let Some(p) = MOTION_PRESETS.get(state.preset)
        {
            out.push(("motion.apply_preset", span_params(view, json!({ "preset": p.name }))));
        }
    });
    curve_section(ui, view, state, out);
}

fn ease_section(ui: &mut Ui, view: &TweenView, state: &mut MotionUi, out: &mut Vec<Cmd>) {
    let mut accel = view.acceleration;
    ui.horizontal(|ui| {
        ui.label("Ease");
        if ui.add(egui::Slider::new(&mut accel, -100..=100)).changed() {
            out.push(("timeline.set_ease", span_params(view, json!({ "acceleration": accel }))));
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Custom ease...").clicked() {
            let (p1, p2) = match view.custom.as_deref() {
                Some([_, a, b, _]) => (*a, *b),
                _ => ([0.3, 0.0], [0.7, 1.0]),
            };
            state.curve = Some(CurveEdit { target: CurveTarget::Tween(EaseTarget::All), p1, p2 });
        }
        if view.custom.is_some() && ui.button("Remove custom ease").clicked() {
            out.push(("timeline.set_custom_ease", span_params(view, json!({ "curve": Value::Null }))));
        }
    });
    if view.kind == Kind::Shape {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Add Shape Hint").clicked() {
                out.push(("shape.add_hint", span_params(view, json!({}))));
            }
            if ui.add_enabled(!view.hints.is_empty(), egui::Button::new("Remove All Hints")).clicked() {
                out.push(("shape.remove_hint", span_params(view, json!({}))));
            }
        });
        for h in &view.hints {
            ui.horizontal(|ui| {
                ui.label(format!("Hint {}", hint_letter(h.id)));
                if ui.small_button("Remove").clicked() {
                    out.push(("shape.remove_hint", span_params(view, json!({ "hint": h.id }))));
                }
            });
        }
    }
    curve_section(ui, view, state, out);
}

fn curve_section(ui: &mut Ui, view: &TweenView, state: &mut MotionUi, out: &mut Vec<Cmd>) {
    let Some(edit) = state.curve.as_mut() else { return };
    ui.separator();
    ui.label("Custom ease");
    bezier_editor(ui, edit);
    let mut close = false;
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            let curve = curve_points(edit);
            let command = match edit.target {
                CurveTarget::Key(prop, offset) => {
                    ("motion.set_key_ease", span_params(view, json!({ "property": prop.label(), "offset": offset, "curve": curve })))
                }
                CurveTarget::Tween(target) => ("timeline.set_custom_ease", span_params(view, json!({ "target": target.xfl_name(), "curve": curve }))),
            };
            out.push(command);
            close = true;
        }
        if ui.button("Cancel").clicked() {
            close = true;
        }
    });
    if close {
        state.curve = None;
    }
}

/// The letter of a hint id (0 = a).
#[must_use]
pub fn hint_letter(id: u32) -> char {
    u8::try_from(id).ok().and_then(|n| b'a'.checked_add(n)).map_or('?', char::from)
}

/// What can be grabbed on the stage for the keyframe under the playhead, with stage positions.
#[must_use]
pub fn handles(doc: &AniDoc) -> Vec<(Handle, Pos2)> {
    let Some(view) = tween_view(doc) else { return Vec::new() };
    let state = &doc.editor.state;
    let Some(layer) = doc.document().timeline(&state.timeline).and_then(|t| t.layers.get(state.layer)) else { return Vec::new() };
    let mut out = Vec::new();
    if view.kind == Kind::Motion {
        let xs = view.tracks.iter().find(|t| t.prop == MotionProp::X);
        let ys = view.tracks.iter().find(|t| t.prop == MotionProp::Y);
        if let (Some(xs), Some(ys)) = (xs, ys) {
            let mut offsets: Vec<u32> = xs.keys.iter().chain(&ys.keys).map(|k| k.offset).collect();
            offsets.sort_unstable();
            offsets.dedup();
            for o in offsets {
                let index = num::u32_to_usize(o);
                if let (Some(x), Some(y)) = (xs.samples.get(index), ys.samples.get(index)) {
                    out.push((Handle::Path(o), pos2(num::f64_to_f32(*x), num::f64_to_f32(*y))));
                }
            }
        }
    }
    let point = |p: anim_core::Point| pos2(num::f64_to_f32(p.x), num::f64_to_f32(p.y));
    if view.kind == Kind::Shape && state.frame == view.start {
        out.extend(view.hints.iter().map(|h| (Handle::HintStart(h.id), point(h.start))));
    }
    // The end keyframe of a shape tween shows the end points of the hints of the keyframe before it.
    if let Some(previous) = layer.frames.iter().filter(|f| f.index < state.frame).max_by_key(|f| f.index)
        && previous.tween_type == TweenType::Shape
        && layer.next_keyframe_after(previous.index).is_some_and(|n| n.index == state.frame)
    {
        out.extend(previous.shape_hints.iter().map(|h| (Handle::HintEnd { tween: previous.index.saturating_add(1), hint: h.id }, point(h.end))));
    }
    out
}

/// The overlays for [`handles`], with the one being dragged shown where the pointer is.
#[must_use]
pub fn overlays(doc: &AniDoc, drag: Option<Drag>, accent: Color32) -> Vec<Overlay> {
    let items = handles(doc);
    let mut out = Vec::new();
    if let Some(view) = tween_view(doc)
        && view.kind == Kind::Motion
    {
        let xs = view.tracks.iter().find(|t| t.prop == MotionProp::X);
        let ys = view.tracks.iter().find(|t| t.prop == MotionProp::Y);
        if let (Some(xs), Some(ys)) = (xs, ys) {
            let line: Vec<Pos2> = xs.samples.iter().zip(&ys.samples).map(|(x, y)| pos2(num::f64_to_f32(*x), num::f64_to_f32(*y))).collect();
            if line.len() > 1 {
                out.push(Overlay::Polygon { points: line.clone(), closed: false, style: OverlayStyle::line(accent) });
                // Every frame of the path is a small dot, the keys are diamonds.
                out.extend(line.into_iter().map(|pos| Overlay::Marker { pos, shape: MarkerShape::Circle, color: accent, filled: true }));
            }
        }
    }
    for (handle, pos) in items {
        let at = match drag {
            Some(d) if d.handle == handle => d.pos,
            _ => pos,
        };
        match handle {
            Handle::Path(_) => out.push(Overlay::Marker { pos: at, shape: MarkerShape::Diamond, color: Color32::WHITE, filled: true }),
            Handle::HintStart(id) | Handle::HintEnd { hint: id, .. } => {
                let colour = if matches!(handle, Handle::HintStart(_)) { Color32::from_rgb(230, 190, 40) } else { Color32::from_rgb(60, 200, 90) };
                out.push(Overlay::Marker { pos: at, shape: MarkerShape::Circle, color: colour, filled: true });
                out.push(Overlay::Text { pos: pos2(at.x + 6.0, at.y - 6.0), text: hint_letter(id).to_string(), color: colour });
            }
        }
    }
    out
}

/// The handle under `pos` (stage coordinates), if any, within a grab radius of nine screen points.
#[must_use]
pub fn pick(items: &[(Handle, Pos2)], pos: Pos2, zoom: f32) -> Option<Handle> {
    let radius = 9.0 / zoom.max(0.01);
    // The first point of a path is the object itself.
    items
        .iter()
        .filter(|(h, p)| *h != Handle::Path(0) && p.distance(pos) <= radius)
        .min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos)))
        .map(|(h, _)| *h)
}

/// The command that drops `drag` at its position, for the keyframe the view describes.
#[must_use]
pub fn drop_command(view: &TweenView, handle: Handle, pos: Pos2) -> Cmd {
    let (x, y) = (f64::from(pos.x), f64::from(pos.y));
    match handle {
        Handle::Path(offset) => ("motion.set_path_point", span_params(view, json!({ "offset": offset, "x": x, "y": y }))),
        Handle::HintStart(hint) => ("shape.move_hint", span_params(view, json!({ "hint": hint, "end": false, "x": x, "y": y }))),
        // The end points live on the tween's start keyframe; the playhead is on the end keyframe.
        Handle::HintEnd { tween, hint } => ("shape.move_hint", params(view, json!({ "frame": tween, "hint": hint, "end": true, "x": x, "y": y }))),
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Affine;
    use anim_scene::{Element, Frame, SymbolInstance};
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        let mut doc = AniDoc::default();
        let instance = SymbolInstance::new("Ball", Affine::translate((10.0, 20.0)));
        if let Some(layer) = doc.editor.document.scenes.first_mut().and_then(|s| s.layers.first_mut()) {
            layer.frames = vec![Frame::new(0, 11, vec![Element::SymbolInstance(Box::new(instance))])];
        }
        Session::new(doc, crate::commands::registry())
    }

    #[test]
    fn a_motion_tween_shows_its_tracks_and_path_handles() {
        let mut s = session();
        assert_eq!(tween_view(s.doc()).unwrap().kind, Kind::None);
        s.execute("timeline.create_motion_tween", &json!({ "layer": 0, "frame": 1 })).unwrap();
        s.execute("motion.set_key", &json!({ "layer": 0, "frame": 1, "property": "x", "offset": 10, "value": 110 })).unwrap();
        s.execute("motion.set_path_point", &json!({ "layer": 0, "frame": 1, "offset": 5, "x": 60, "y": 90 })).unwrap();
        s.doc_mut().editor.state.frame = 5;
        let view = tween_view(s.doc()).unwrap();
        assert_eq!((view.kind, view.span, view.offset), (Kind::Motion, 11, 5));
        let x = view.tracks.iter().find(|t| t.prop == MotionProp::X).unwrap();
        assert!((x.now - 60.0).abs() < 1e-9, "{}", x.now);
        assert_eq!(x.samples.len(), 11);
        let items = handles(s.doc());
        assert_eq!(items.iter().filter(|(h, _)| matches!(h, Handle::Path(_))).count(), 3, "start, the point at 5 and the end");
        let at_point = items.iter().find(|(h, _)| *h == Handle::Path(5)).unwrap().1;
        assert_eq!(pick(&items, at_point, 1.0), Some(Handle::Path(5)));
        assert_eq!(pick(&items, pos2(500.0, 500.0), 1.0), None);
        assert_eq!(pick(&items, items.iter().find(|(h, _)| *h == Handle::Path(0)).unwrap().1, 1.0), None, "the first point is the object");
        let (id, p) = drop_command(&view, Handle::Path(5), pos2(70.0, 80.0));
        s.execute(id, &p).unwrap();
        let moved = tween_view(s.doc()).unwrap();
        assert!((moved.tracks.iter().find(|t| t.prop == MotionProp::Y).unwrap().now - 80.0).abs() < 1e-9);
        assert!(!overlays(s.doc(), None, Color32::WHITE).is_empty());
    }

    #[test]
    fn shape_hints_have_start_and_end_handles() {
        let mut s = session();
        if let Some(layer) = s.doc_mut().editor.document.scenes.first_mut().and_then(|sc| sc.layers.first_mut()) {
            let mut start = Frame::new(0, 5, vec![]);
            start.tween_type = TweenType::Shape;
            layer.frames = vec![start, Frame::new(5, 1, vec![])];
        }
        s.execute("shape.add_hint", &json!({ "layer": 0, "frame": 1 })).unwrap();
        s.doc_mut().editor.state.frame = 0;
        let items = handles(s.doc());
        assert!(matches!(items.as_slice(), [(Handle::HintStart(0), _)]));
        s.doc_mut().editor.state.frame = 5;
        let items = handles(s.doc());
        assert!(matches!(items.as_slice(), [(Handle::HintEnd { tween: 1, hint: 0 }, _)]));
        let view = tween_view(s.doc()).unwrap();
        let (id, p) = drop_command(&view, Handle::HintEnd { tween: 1, hint: 0 }, pos2(33.0, 44.0));
        s.execute(id, &p).unwrap();
        let hint = s.doc().editor.document.scenes[0].layers[0].frames[0].shape_hints[0];
        assert_eq!((hint.end.x, hint.end.y), (33.0, 44.0));
        assert_eq!(hint_letter(2), 'c');
    }
}
