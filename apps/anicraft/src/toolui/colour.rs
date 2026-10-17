//! The Colour panel: stroke and fill type, RGB, HSB, hex and alpha, gradient stops, overflow modes, bitmap fills and
//! the stroke style (weight, caps, joins, miter limit, scale mode).

use anicraft_tools::ToolSettings;
use anim_core::{Affine, Rgba8};
use anim_display::{Cap, GradientStop, Interpolation, Join, Spread, StrokeScale};
use anim_shape::{BitmapFill, FillStyle, GradientFill, StrokeStyle};
use anim_ui::theme::current_palette;
use egui::ecolor::Hsva;
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use super::toolbox::paint_fill;
use super::{ColourEdit, ColourTarget, ToolUi, ToolUiEvent};
use crate::view::to_color32;

/// The most stops a gradient has.
const MAX_STOPS: usize = 15;
/// The scale of a bitmap shown at 100 percent in an XFL fill matrix.
const BITMAP_FULL: f64 = 20.0;

/// The kinds of fill in the type menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillKind {
    None,
    Solid,
    Linear,
    Radial,
    Bitmap,
}

impl FillKind {
    const ALL: [Self; 5] = [Self::None, Self::Solid, Self::Linear, Self::Radial, Self::Bitmap];

    const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Solid => "Solid colour",
            Self::Linear => "Linear gradient",
            Self::Radial => "Radial gradient",
            Self::Bitmap => "Bitmap fill",
        }
    }
}

/// The menu entry that describes `fill`.
#[must_use]
pub const fn kind_of(fill: &FillStyle, enabled: bool) -> FillKind {
    if !enabled {
        return FillKind::None;
    }
    match fill {
        FillStyle::Solid(_) => FillKind::Solid,
        FillStyle::LinearGradient(_) => FillKind::Linear,
        FillStyle::RadialGradient(_) => FillKind::Radial,
        FillStyle::Bitmap(_) => FillKind::Bitmap,
    }
}

/// The first colour of a fill, what a solid fill becomes when the type changes.
fn first_colour(fill: &FillStyle) -> Rgba8 {
    match fill {
        FillStyle::Solid(c) => *c,
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => g.stops.first().map_or(Rgba8::BLACK, |s| s.color),
        FillStyle::Bitmap(_) => Rgba8::BLACK,
    }
}

/// `fill` turned into another kind: gradients keep their stops when switching between linear and radial, a solid
/// colour becomes a gradient from that colour to white, a bitmap fill needs a library bitmap.
#[must_use]
pub fn convert_fill(fill: &FillStyle, to: FillKind, bitmap: Option<&str>) -> Option<FillStyle> {
    let gradient_of = |fill: &FillStyle| match fill {
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => g.clone(),
        other => GradientFill {
            stops: vec![GradientStop { offset: 0.0, color: first_colour(other) }, GradientStop { offset: 1.0, color: Rgba8::WHITE }],
            ..GradientFill::default()
        },
    };
    match to {
        FillKind::None => None,
        FillKind::Solid => Some(FillStyle::Solid(first_colour(fill))),
        FillKind::Linear => Some(FillStyle::LinearGradient(gradient_of(fill))),
        FillKind::Radial => Some(FillStyle::RadialGradient(gradient_of(fill))),
        FillKind::Bitmap => match fill {
            FillStyle::Bitmap(_) => Some(fill.clone()),
            _ => {
                bitmap.map(|path| FillStyle::Bitmap(BitmapFill { path: path.to_owned(), matrix: Affine::scale(BITMAP_FULL), clipped: false, smoothing: true }))
            }
        },
    }
}

/// The colour of a gradient at `t` (0 to 1).
#[must_use]
pub fn sample(stops: &[GradientStop], t: f32) -> Rgba8 {
    if stops.is_empty() { Rgba8::BLACK } else { GradientStop::color_at(stops, t) }
}

/// What the fill editor wants done.
#[derive(Debug, Clone, PartialEq)]
enum FillAction {
    /// Nothing changed, but the gesture ended.
    Keep,
    Set(FillStyle),
    Off,
}

/// Draws the Colour panel.
pub fn colour_panel(ui: &mut Ui, s: &ToolSettings, bitmaps: &[String], st: &mut ToolUi) -> Vec<ToolUiEvent> {
    let mut events = Vec::new();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut st.target, ColourTarget::Stroke, "Stroke");
        ui.selectable_value(&mut st.target, ColourTarget::Fill, "Fill");
    });
    ui.separator();
    match st.target {
        ColourTarget::Fill => {
            if let Some((action, commit)) = fill_editor(ui, st, "fill", &s.fill, s.fill_enabled, bitmaps) {
                events.push(match action {
                    FillAction::Keep => ToolUiEvent::Colour(ColourEdit { fill: None, stroke: None, commit }),
                    FillAction::Set(fill) => ToolUiEvent::Colour(ColourEdit { fill: Some(fill), stroke: None, commit }),
                    FillAction::Off => ToolUiEvent::options(json!({ "fill_enabled": false })),
                });
            }
        }
        ColourTarget::Stroke => {
            if let Some(edit) = stroke_style(ui, &s.stroke) {
                events.push(ToolUiEvent::Colour(edit));
            }
            ui.separator();
            if let Some((action, commit)) = fill_editor(ui, st, "stroke", &s.stroke.solid().fill, s.stroke_enabled, bitmaps) {
                events.push(match action {
                    FillAction::Keep => ToolUiEvent::Colour(ColourEdit { fill: None, stroke: None, commit }),
                    FillAction::Set(fill) => {
                        let mut stroke = s.stroke.clone();
                        stroke.solid_mut().fill = fill;
                        ToolUiEvent::Colour(ColourEdit { fill: None, stroke: Some(stroke), commit })
                    }
                    FillAction::Off => ToolUiEvent::options(json!({ "stroke_enabled": false })),
                });
            }
        }
    }
    events
}

// ---------------------------------------------------------------------------------------------------------------
// Stroke style
// ---------------------------------------------------------------------------------------------------------------

const fn cap_label(cap: Cap) -> &'static str {
    match cap {
        Cap::None => "None",
        Cap::Round => "Round",
        Cap::Square => "Square",
    }
}

const fn join_label(join: Join) -> &'static str {
    match join {
        Join::Miter => "Miter",
        Join::Round => "Round",
        Join::Bevel => "Bevel",
    }
}

const fn scale_label(scale: StrokeScale) -> &'static str {
    match scale {
        StrokeScale::Normal => "Normal",
        StrokeScale::Horizontal => "Horizontal",
        StrokeScale::Vertical => "Vertical",
        StrokeScale::None => "None",
    }
}

/// Weight, caps, joins, miter limit, scale mode and pixel hinting of the toolbox stroke.
fn stroke_style(ui: &mut Ui, stroke: &StrokeStyle) -> Option<ColourEdit> {
    let mut next = stroke.clone();
    let mut changed = false;
    let mut commit = false;
    {
        let solid = next.solid_mut();
        egui::Grid::new("stroke-style").num_columns(2).show(ui, |ui| {
            ui.label("Width");
            let r = ui.add(egui::DragValue::new(&mut solid.weight).range(0.0..=200.0).speed(0.1).suffix(" px"));
            changed |= r.changed();
            commit |= r.drag_stopped() || r.lost_focus();
            ui.end_row();
            ui.label("Cap");
            egui::ComboBox::from_id_salt("stroke-cap").selected_text(cap_label(solid.caps)).show_ui(ui, |ui| {
                for cap in [Cap::None, Cap::Round, Cap::Square] {
                    if ui.selectable_value(&mut solid.caps, cap, cap_label(cap)).clicked() {
                        changed = true;
                        commit = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Join");
            egui::ComboBox::from_id_salt("stroke-join").selected_text(join_label(solid.joints)).show_ui(ui, |ui| {
                for join in [Join::Miter, Join::Round, Join::Bevel] {
                    if ui.selectable_value(&mut solid.joints, join, join_label(join)).clicked() {
                        changed = true;
                        commit = true;
                    }
                }
            });
            ui.end_row();
            if solid.joints == Join::Miter {
                ui.label("Miter");
                let r = ui.add(egui::DragValue::new(&mut solid.miter_limit).range(1.0..=60.0).speed(0.1));
                changed |= r.changed();
                commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            ui.label("Scale");
            egui::ComboBox::from_id_salt("stroke-scale").selected_text(scale_label(solid.scale_mode)).show_ui(ui, |ui| {
                for scale in [StrokeScale::Normal, StrokeScale::Horizontal, StrokeScale::Vertical, StrokeScale::None] {
                    if ui.selectable_value(&mut solid.scale_mode, scale, scale_label(scale)).clicked() {
                        changed = true;
                        commit = true;
                    }
                }
            });
            ui.end_row();
            ui.label("Hinting");
            if ui.checkbox(&mut solid.pixel_hinting, "Stroke hinting").changed() {
                changed = true;
                commit = true;
            }
            ui.end_row();
        });
    }
    if changed {
        Some(ColourEdit { fill: None, stroke: Some(next), commit })
    } else if commit {
        Some(ColourEdit { fill: None, stroke: None, commit })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Fill editor
// ---------------------------------------------------------------------------------------------------------------

fn fill_editor(ui: &mut Ui, st: &mut ToolUi, id: &str, fill: &FillStyle, enabled: bool, bitmaps: &[String]) -> Option<(FillAction, bool)> {
    let mut out: Option<(FillAction, bool)> = None;
    let current = kind_of(fill, enabled);
    let mut chosen = current;
    ui.horizontal(|ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt(("fill-kind", id)).selected_text(current.label()).show_ui(ui, |ui| {
            for kind in FillKind::ALL {
                if kind == FillKind::Bitmap && bitmaps.is_empty() {
                    ui.add_enabled(false, egui::Button::new("Bitmap fill (import a bitmap first)"));
                    continue;
                }
                ui.selectable_value(&mut chosen, kind, kind.label());
            }
        });
    });
    if chosen != current {
        let action = convert_fill(fill, chosen, bitmaps.first().map(String::as_str)).map_or(FillAction::Off, FillAction::Set);
        st.stop = 0;
        return Some((action, true));
    }
    if !enabled {
        ui.weak("This colour is off. Pick a type to turn it on.");
        return None;
    }
    ui.add_space(4.0);
    match fill {
        FillStyle::Solid(c) => {
            if let Some((new, commit)) = colour_fields(ui, st, *c) {
                out = Some((if new == *c { FillAction::Keep } else { FillAction::Set(FillStyle::Solid(new)) }, commit));
            }
        }
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => {
            let radial = matches!(fill, FillStyle::RadialGradient(_));
            if let Some((new, commit)) = gradient_editor(ui, st, g, radial) {
                let style = if radial { FillStyle::RadialGradient(new) } else { FillStyle::LinearGradient(new) };
                out = Some((if style == *fill { FillAction::Keep } else { FillAction::Set(style) }, commit));
            }
        }
        FillStyle::Bitmap(b) => {
            if let Some(new) = bitmap_editor(ui, b, bitmaps) {
                out = Some((FillAction::Set(FillStyle::Bitmap(new)), true));
            }
        }
    }
    out
}

fn bitmap_editor(ui: &mut Ui, b: &BitmapFill, bitmaps: &[String]) -> Option<BitmapFill> {
    let mut next = b.clone();
    let mut changed = false;
    egui::ComboBox::from_id_salt("fill-bitmap").selected_text(next.path.clone()).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
        for name in bitmaps {
            if ui.selectable_label(next.path == *name, name).clicked() {
                name.clone_into(&mut next.path);
                changed = true;
            }
        }
    });
    changed |= ui.checkbox(&mut next.clipped, "Clipped (do not tile)").changed();
    changed |= ui.checkbox(&mut next.smoothing, "Smoothed").changed();
    changed.then_some(next)
}

/// RGB, HSB, hex and alpha of one colour. Returns the new colour and whether the edit gesture ended.
fn colour_fields(ui: &mut Ui, st: &mut ToolUi, c: Rgba8) -> Option<(Rgba8, bool)> {
    let mut rgb = [i32::from(c.r), i32::from(c.g), i32::from(c.b)];
    let mut alpha = f32::from(c.a) / 255.0 * 100.0;
    if st.hsb.is_none_or(|(rgba, _)| rgba != c) {
        let h = Hsva::from_srgba_unmultiplied([c.r, c.g, c.b, 255]);
        st.hsb = Some((c, [h.h, h.s, h.v]));
    }
    let mut hsb = st.hsb.map_or([0.0; 3], |(_, v)| v);
    if st.hex_for != Some(c) {
        st.hex = c.to_hex_rgb();
        st.hex_for = Some(c);
    }
    let mut commit = false;
    let mut rgb_changed = false;
    let mut hsb_changed = false;
    let mut alpha_changed = false;
    let mut hex_value: Option<Rgba8> = None;

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::hover());
        paint_fill(ui.painter(), rect, &FillStyle::Solid(c));
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, current_palette(ui.ctx()).border), StrokeKind::Outside);
        let r = ui.add(egui::TextEdit::singleline(&mut st.hex).desired_width(72.0).char_limit(9));
        if r.lost_focus() {
            commit = true;
            match Rgba8::from_hex(&st.hex) {
                Ok(parsed) => hex_value = Some(Rgba8::new(parsed.r, parsed.g, parsed.b, c.a)),
                Err(_) => st.hex_for = None,
            }
        }
    });
    ui.spacing_mut().slider_width = (ui.available_width() - 90.0).clamp(60.0, 200.0);
    egui::Grid::new("colour-fields").num_columns(2).show(ui, |ui| {
        for (label, value) in ["R", "G", "B"].into_iter().zip(rgb.iter_mut()) {
            ui.label(label);
            let r = ui.add(egui::Slider::new(value, 0..=255));
            rgb_changed |= r.changed();
            commit |= r.drag_stopped() || r.lost_focus();
            ui.end_row();
        }
        ui.label("H");
        let r = ui.add(
            egui::Slider::new(&mut hsb[0], 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}", v * 360.0))
                .custom_parser(|t| t.parse::<f64>().ok().map(|v| v / 360.0)),
        );
        hsb_changed |= r.changed();
        commit |= r.drag_stopped() || r.lost_focus();
        ui.end_row();
        ui.label("S");
        let r = ui.add(
            egui::Slider::new(&mut hsb[1], 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}", v * 100.0))
                .custom_parser(|t| t.parse::<f64>().ok().map(|v| v / 100.0)),
        );
        hsb_changed |= r.changed();
        commit |= r.drag_stopped() || r.lost_focus();
        ui.end_row();
        ui.label("B");
        let r = ui.add(
            egui::Slider::new(&mut hsb[2], 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}", v * 100.0))
                .custom_parser(|t| t.parse::<f64>().ok().map(|v| v / 100.0)),
        );
        hsb_changed |= r.changed();
        commit |= r.drag_stopped() || r.lost_focus();
        ui.end_row();
        ui.label("A");
        let r = ui.add(egui::Slider::new(&mut alpha, 0.0..=100.0).suffix("%").fixed_decimals(0));
        alpha_changed |= r.changed();
        commit |= r.drag_stopped() || r.lost_focus();
        ui.end_row();
    });

    let to_u8 = |v: i32| u8::try_from(v.clamp(0, 255)).unwrap_or(u8::MAX);
    let a = anim_core::num::unit_f32_to_u8(alpha / 100.0);
    let new = if let Some(hex) = hex_value {
        hex
    } else if hsb_changed {
        let [r, g, b, _] = Hsva::new(hsb[0], hsb[1], hsb[2], 1.0).to_srgba_unmultiplied();
        st.hsb = Some((Rgba8::new(r, g, b, c.a), hsb));
        Rgba8::new(r, g, b, if alpha_changed { a } else { c.a })
    } else if rgb_changed || alpha_changed {
        Rgba8::new(to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]), if alpha_changed { a } else { c.a })
    } else {
        c
    };
    (new != c || commit).then_some((new, commit))
}

// ---------------------------------------------------------------------------------------------------------------
// Gradient editor
// ---------------------------------------------------------------------------------------------------------------

const BAR_HEIGHT: f32 = 22.0;
const MARKER_HEIGHT: f32 = 14.0;
/// How far below the bar a stop is dragged to delete it.
const DELETE_DISTANCE: f32 = 36.0;

fn sorted(mut g: GradientFill) -> GradientFill {
    g.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    g
}

const fn spread_label(spread: Spread) -> &'static str {
    match spread {
        Spread::Pad => "Extend",
        Spread::Reflect => "Reflect",
        Spread::Repeat => "Repeat",
    }
}

fn gradient_editor(ui: &mut Ui, st: &mut ToolUi, g: &GradientFill, radial: bool) -> Option<(GradientFill, bool)> {
    let mut next = g.clone();
    let mut changed = false;
    let mut commit = false;
    st.stop = st.stop.min(next.stops.len().saturating_sub(1));

    // The bar with the stop markers below it.
    let width = ui.available_width().max(60.0) - 12.0;
    let (area, response) = ui.allocate_exact_size(vec2(width + 12.0, BAR_HEIGHT + MARKER_HEIGHT + 4.0), Sense::click_and_drag());
    let bar = Rect::from_min_size(area.min + vec2(6.0, 0.0), vec2(width, BAR_HEIGHT));
    let p = current_palette(ui.ctx());
    let preview = FillStyle::LinearGradient(g.clone());
    paint_fill(ui.painter(), bar, &preview);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, p.border), StrokeKind::Outside);
    let x_of = |offset: f32| bar.left() + bar.width() * offset;
    for (i, stop) in g.stops.iter().enumerate() {
        let x = x_of(stop.offset);
        let top = bar.bottom() + 1.0;
        let tri = vec![pos2(x, top), pos2(x - 5.0, top + MARKER_HEIGHT), pos2(x + 5.0, top + MARKER_HEIGHT)];
        let selected = i == st.stop;
        ui.painter().add(egui::Shape::convex_polygon(
            tri,
            to_color32(stop.color).to_opaque(),
            Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { p.accent } else { p.text }),
        ));
    }
    let offset_at = |x: f32| ((x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0);
    let pointer = response.interact_pointer_pos();
    if let Some(pos) = pointer {
        if response.drag_started() || response.clicked() {
            let nearest = g.stops.iter().enumerate().min_by(|(_, a), (_, b)| (x_of(a.offset) - pos.x).abs().total_cmp(&(x_of(b.offset) - pos.x).abs()));
            match nearest {
                Some((i, stop)) if (x_of(stop.offset) - pos.x).abs() <= 7.0 => st.stop = i,
                _ if next.stops.len() < MAX_STOPS && pos.y <= bar.bottom() + MARKER_HEIGHT + 2.0 => {
                    let offset = offset_at(pos.x);
                    next.stops.push(GradientStop { offset, color: sample(&g.stops, offset) });
                    next = sorted(next);
                    st.stop = next.stops.iter().position(|s| (s.offset - offset).abs() < f32::EPSILON).unwrap_or(0);
                    changed = true;
                }
                _ => {}
            }
        }
        if response.dragged() && !changed {
            let far = pos.y > bar.bottom() + DELETE_DISTANCE && next.stops.len() > 2;
            if let Some(stop) = next.stops.get(st.stop).copied().filter(|_| !far) {
                let offset = offset_at(pos.x);
                if (stop.offset - offset).abs() > f32::EPSILON {
                    if let Some(slot) = next.stops.get_mut(st.stop) {
                        slot.offset = offset;
                    }
                    let moved = next.stops.get(st.stop).copied();
                    next = sorted(next);
                    if let Some(moved) = moved {
                        st.stop = next.stops.iter().position(|s| s.offset.to_bits() == moved.offset.to_bits() && s.color == moved.color).unwrap_or(st.stop);
                    }
                    changed = true;
                }
            }
        }
    }
    if response.drag_stopped() {
        commit = true;
        let pulled_away = response.interact_pointer_pos().is_some_and(|pos| pos.y > bar.bottom() + DELETE_DISTANCE);
        if pulled_away && next.stops.len() > 2 && st.stop < next.stops.len() {
            next.stops.remove(st.stop);
            st.stop = st.stop.saturating_sub(1);
            changed = true;
        }
    }
    if response.clicked() {
        commit = true;
    }
    ui.weak("Click the bar to add a stop. Drag a stop to move it, or pull it away from the bar to delete it.");

    // The selected stop.
    if let Some(stop) = next.stops.get(st.stop).copied() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(format!("Stop {}", st.stop.saturating_add(1)));
            let mut percent = f64::from(stop.offset) * 100.0;
            let r = ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).suffix("%"));
            if r.changed() {
                if let Some(slot) = next.stops.get_mut(st.stop) {
                    slot.offset = anim_core::num::f64_to_f32(percent / 100.0);
                }
                let moved = next.stops.get(st.stop).copied();
                next = sorted(next.clone());
                if let Some(moved) = moved {
                    st.stop = next.stops.iter().position(|s| s.offset.to_bits() == moved.offset.to_bits() && s.color == moved.color).unwrap_or(st.stop);
                }
                changed = true;
            }
            commit |= r.drag_stopped() || r.lost_focus();
            if ui.add_enabled(next.stops.len() > 2, egui::Button::new("Remove")).clicked() {
                next.stops.remove(st.stop);
                st.stop = st.stop.saturating_sub(1);
                changed = true;
                commit = true;
            }
        });
        let edited = next.stops.get(st.stop).copied().and_then(|stop| colour_fields(ui, st, stop.color).map(|r| (stop, r)));
        if let Some((stop, (colour, done))) = edited {
            if let Some(slot) = next.stops.get_mut(st.stop) {
                slot.color = colour;
            }
            changed |= colour != stop.color;
            commit |= done;
        }
    }

    ui.add_space(4.0);
    egui::Grid::new("gradient-options").num_columns(2).show(ui, |ui| {
        ui.label("Overflow");
        egui::ComboBox::from_id_salt("gradient-spread").selected_text(spread_label(next.spread)).show_ui(ui, |ui| {
            for spread in [Spread::Pad, Spread::Reflect, Spread::Repeat] {
                if ui.selectable_value(&mut next.spread, spread, spread_label(spread)).clicked() {
                    changed = true;
                    commit = true;
                }
            }
        });
        ui.end_row();
        ui.label("Colour space");
        let mut linear = next.interpolation == Interpolation::LinearRgb;
        if ui.checkbox(&mut linear, "Linear RGB").changed() {
            next.interpolation = if linear { Interpolation::LinearRgb } else { Interpolation::Rgb };
            changed = true;
            commit = true;
        }
        ui.end_row();
        if radial {
            ui.label("Focal point");
            let r = ui.add(egui::Slider::new(&mut next.focal, -1.0..=1.0).fixed_decimals(2));
            changed |= r.changed();
            commit |= r.drag_stopped() || r.lost_focus();
            ui.end_row();
        }
    });
    let _ = Color32::TRANSPARENT;
    (changed || commit).then_some((next, commit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> FillStyle {
        FillStyle::LinearGradient(GradientFill {
            stops: vec![GradientStop { offset: 0.0, color: Rgba8::BLACK }, GradientStop { offset: 1.0, color: Rgba8::WHITE }],
            ..GradientFill::default()
        })
    }

    #[test]
    fn the_type_menu_converts_between_kinds() {
        let solid = FillStyle::Solid(Rgba8::opaque(200, 10, 10));
        let FillStyle::RadialGradient(g) = convert_fill(&solid, FillKind::Radial, None).unwrap() else { panic!("radial") };
        assert_eq!(g.stops.first().map(|s| s.color), Some(Rgba8::opaque(200, 10, 10)));
        assert_eq!(g.stops.len(), 2);
        // Linear and radial keep the stops.
        let FillStyle::RadialGradient(r) = convert_fill(&gradient(), FillKind::Radial, None).unwrap() else { panic!("radial") };
        assert_eq!(r.stops.len(), 2);
        // A gradient becomes a solid of its first colour.
        assert_eq!(convert_fill(&gradient(), FillKind::Solid, None), Some(FillStyle::Solid(Rgba8::BLACK)));
        assert_eq!(convert_fill(&solid, FillKind::None, None), None);
        assert_eq!(convert_fill(&solid, FillKind::Bitmap, None), None);
        assert!(matches!(convert_fill(&solid, FillKind::Bitmap, Some("a.png")), Some(FillStyle::Bitmap(b)) if b.path == "a.png"));
    }

    #[test]
    fn kinds_follow_the_fill_and_the_switch() {
        assert_eq!(kind_of(&gradient(), true), FillKind::Linear);
        assert_eq!(kind_of(&gradient(), false), FillKind::None);
        assert_eq!(kind_of(&FillStyle::Solid(Rgba8::BLACK), true), FillKind::Solid);
    }

    #[test]
    fn sampling_interpolates_between_stops() {
        let FillStyle::LinearGradient(g) = gradient() else { panic!("gradient") };
        assert_eq!(sample(&g.stops, 0.0), Rgba8::BLACK);
        assert_eq!(sample(&g.stops, 1.0), Rgba8::WHITE);
        let mid = sample(&g.stops, 0.5);
        assert!((i32::from(mid.r) - 128).abs() <= 1, "{mid:?}");
    }
}
