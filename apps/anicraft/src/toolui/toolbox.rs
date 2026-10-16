//! The Tools panel: grouped tool buttons with flyouts, the stroke and fill colour chips and the option switches.

use anicraft_tools::ToolSettings;
use anim_shape::FillStyle;
use anim_ui::icons;
use anim_ui::theme::current_palette;
use anim_ui::{ToolItem, toolbar};
use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;

use super::{ColourTarget, ToolUi, ToolUiEvent, group_head, groups};
use crate::view::to_color32;

const BUTTON: f32 = 30.0;
/// Press time that opens a group's flyout.
const LONG_PRESS: f64 = 0.45;

/// Draws the toolbox and returns what was picked or switched.
pub fn toolbox(ui: &mut Ui, tools: &[ToolItem], active: &str, settings: &ToolSettings, swatches: &[FillStyle], st: &mut ToolUi) -> Vec<ToolUiEvent> {
    let mut events = Vec::new();
    tool_buttons(ui, tools, active, st, &mut events);
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
    colour_block(ui, settings, swatches, st, &mut events);
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);
    switches(ui, settings, &mut events);
    events
}

fn tool_buttons(ui: &mut Ui, tools: &[ToolItem], active: &str, st: &mut ToolUi, events: &mut Vec<ToolUiEvent>) {
    let spacing = ui.spacing().item_spacing.x;
    let per_row = toolbar::columns(ui.available_width(), BUTTON, spacing);
    let all = groups(tools);
    ui.spacing_mut().item_spacing = vec2(spacing, 3.0);
    let mut in_row = 0;
    let mut open_at: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(spacing, 3.0);
        for members in &all {
            let Some(first) = members.first() else { continue };
            if first.separator_before && in_row > 0 {
                ui.end_row();
                ui.add_space(6.0);
                in_row = 0;
            }
            if in_row >= per_row {
                ui.end_row();
                in_row = 0;
            }
            in_row = in_row.saturating_add(1);
            let head = group_head(&first.id).to_owned();
            let current = if members.iter().any(|t| t.id == active) {
                active.to_owned()
            } else {
                st.group_tool.get(&head).filter(|id| members.iter().any(|t| t.id == **id)).cloned().unwrap_or_else(|| first.id.clone())
            };
            let shown = members.iter().find(|t| t.id == current).unwrap_or(first);
            let grouped = members.len() > 1;
            let (response, on_corner) = group_button(ui, shown, members.iter().any(|t| t.id == active), grouped);
            let long = grouped
                && response.is_pointer_button_down_on()
                && ui.input(|i| i.pointer.press_start_time()).is_some_and(|t| ui.input(|i| i.time) - t > LONG_PRESS);
            if grouped && (response.secondary_clicked() || long || (response.clicked() && on_corner)) {
                open_at = Some(head);
            } else if response.clicked() && shown.enabled {
                st.group_tool.insert(head, shown.id.clone());
                events.push(ToolUiEvent::Tool(shown.id.clone()));
            }
            if long {
                ui.ctx().request_repaint();
            }
        }
    });
    if let Some(head) = open_at {
        st.flyout = if st.flyout.as_deref() == Some(head.as_str()) { None } else { Some(head) };
    }
    flyout(ui, &all, active, st, events);
}

/// One toolbox button; `grouped` buttons get a corner mark. Returns the response and whether the press was on the mark.
fn group_button(ui: &mut Ui, tool: &ToolItem, active: bool, grouped: bool) -> (Response, bool) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), if tool.enabled { Sense::click() } else { Sense::hover() });
    let on_corner = grouped && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x > rect.right() - 10.0 && p.y > rect.bottom() - 10.0);
    if ui.is_rect_visible(rect) {
        let p = current_palette(ui.ctx());
        let painter = ui.painter();
        if active {
            painter.rect(rect, 3.0, p.accent_soft, Stroke::new(1.0, p.accent), StrokeKind::Inside);
        } else if response.hovered() && tool.enabled {
            painter.rect_filled(rect, 3.0, p.grid_line_strong);
        }
        let colour = if tool.enabled { p.text } else { p.text_weak.gamma_multiply(0.5) };
        paint_tool(painter, rect.shrink(BUTTON * 0.2), tool, colour);
        if grouped {
            let corner = rect.right_bottom();
            painter.add(egui::Shape::convex_polygon(
                vec![corner - vec2(1.0, 6.0), corner - vec2(6.0, 1.0), corner - vec2(1.0, 1.0)],
                p.text_weak,
                Stroke::NONE,
            ));
        }
    }
    let tip = tool.shortcut.map_or_else(|| tool.label.clone(), |c| format!("{} ({})", tool.label, c.to_ascii_uppercase()));
    (response.on_hover_text(anim_ui::theme::visual(&tip)), on_corner)
}

/// An icon: the tools the shared icon set lacks are drawn here.
fn paint_tool(painter: &egui::Painter, rect: Rect, tool: &ToolItem, colour: Color32) {
    let at = |x: f32, y: f32| pos2(rect.left() + rect.width() * x, rect.top() + rect.height() * y);
    let stroke = Stroke::new(1.5, colour);
    match tool.id.as_str() {
        "gradient_transform" => {
            painter.rect_stroke(Rect::from_min_max(at(0.1, 0.2), at(0.9, 0.8)), 1.0, stroke, StrokeKind::Middle);
            for i in 1..5_u8 {
                let x = f32::from(i) / 5.0;
                painter.line_segment([at(0.1 + 0.8 * x, 0.2), at(0.1 + 0.8 * x, 0.8)], Stroke::new(1.0, colour.gamma_multiply(0.3 + 0.12 * f32::from(i))));
            }
            painter.line_segment([at(0.1, 0.5), at(0.9, 0.5)], stroke);
            painter.circle_filled(at(0.9, 0.5), 2.5, colour);
        }
        "ink" => {
            painter.add(egui::Shape::closed_line(vec![at(0.3, 0.35), at(0.7, 0.35), at(0.85, 0.9), at(0.15, 0.9)], stroke));
            painter.rect_stroke(Rect::from_min_max(at(0.4, 0.1), at(0.6, 0.35)), 1.0, stroke, StrokeKind::Middle);
            painter.line_segment([at(0.25, 0.65), at(0.75, 0.65)], stroke);
        }
        _ => icons::paint(painter, rect, tool.icon, colour),
    }
}

/// The popup listing a group's tools.
fn flyout(ui: &Ui, all: &[Vec<&ToolItem>], active: &str, st: &mut ToolUi, events: &mut Vec<ToolUiEvent>) {
    let Some(open) = st.flyout.clone() else { return };
    let Some(members) = all.iter().find(|m| m.first().is_some_and(|f| group_head(&f.id) == open)) else {
        st.flyout = None;
        return;
    };
    let ctx = ui.ctx().clone();
    let anchor = ui.min_rect().right_top() + vec2(4.0, 0.0);
    let mut chosen: Option<String> = None;
    let area = egui::Area::new(Id::new(("toolbox-flyout", &open))).order(egui::Order::Foreground).fixed_pos(anchor).show(&ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            for tool in members {
                let label = tool.shortcut.map_or_else(|| tool.label.clone(), |c| format!("{}   {}", tool.label, c.to_ascii_uppercase()));
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    paint_tool(ui.painter(), rect, tool, current_palette(ui.ctx()).text);
                    if ui.selectable_label(tool.id == active, anim_ui::theme::visual(&label)).clicked() {
                        chosen = Some(tool.id.clone());
                    }
                });
            }
        });
    });
    if let Some(id) = chosen {
        st.group_tool.insert(open, id.clone());
        st.flyout = None;
        events.push(ToolUiEvent::Tool(id));
    } else if ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| area.response.rect.contains(p)) {
        // A press elsewhere closes the flyout (a press on a group button re-opens it the same frame).
        let on_button = ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < anchor.x);
        if !on_button {
            st.flyout = None;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------------------------------------------

fn chip_colour(fill: &FillStyle) -> Color32 {
    match fill {
        FillStyle::Solid(c) => to_color32(*c),
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => g.stops.first().map_or(Color32::GRAY, |s| to_color32(s.color)),
        FillStyle::Bitmap(_) => Color32::from_gray(150),
    }
}

fn checker(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let cell = 4.0;
    let mut y = rect.top();
    let mut row = false;
    while y < rect.bottom() {
        let mut x = rect.left() + if row { cell } else { 0.0 };
        while x < rect.right() {
            let r = Rect::from_min_size(pos2(x, y), vec2(cell, cell)).intersect(rect);
            painter.rect_filled(r, 0.0, Color32::from_gray(200));
            x += cell * 2.0;
        }
        y += cell;
        row = !row;
    }
}

/// Paints a chip showing `fill`; a gradient shows its stops side by side.
pub(super) fn paint_fill(painter: &egui::Painter, rect: Rect, fill: &FillStyle) {
    checker(painter, rect);
    match fill {
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) if g.stops.len() > 1 => {
            let steps = 12_u8;
            let width = rect.width() / f32::from(steps);
            for i in 0..steps {
                let t = (f32::from(i) + 0.5) / f32::from(steps);
                let colour = super::colour::sample(&g.stops, t);
                let x = rect.left() + width * f32::from(i);
                painter.rect_filled(Rect::from_min_size(pos2(x, rect.top()), vec2(width + 0.5, rect.height())), 0.0, to_color32(colour));
            }
        }
        FillStyle::Solid(c) => {
            painter.rect_filled(rect, 0.0, to_color32(*c));
        }
        other => {
            painter.rect_filled(rect, 0.0, chip_colour(other));
        }
    }
}

fn colour_block(ui: &mut Ui, s: &ToolSettings, swatches: &[FillStyle], st: &mut ToolUi, events: &mut Vec<ToolUiEvent>) {
    let p = current_palette(ui.ctx());
    let size = vec2(52.0, 40.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let stroke_rect = Rect::from_min_size(rect.min, vec2(24.0, 24.0));
    let fill_rect = Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(24.0, 24.0));
    let stroke_fill = s.stroke.solid().fill.clone();
    let chip_ids = [(ColourTarget::Stroke, stroke_rect, &stroke_fill, s.stroke_enabled), (ColourTarget::Fill, fill_rect, &s.fill, s.fill_enabled)];
    let mut clicked: Option<ColourTarget> = None;
    for (target, chip, fill, enabled) in chip_ids {
        let id = ui.id().with(("chip", matches!(target, ColourTarget::Fill)));
        let response = ui.interact(chip, id, Sense::click());
        let painter = ui.painter();
        match target {
            ColourTarget::Stroke => {
                // A stroke chip is a frame: the colour is the border, the middle shows the panel.
                painter.rect_filled(chip, 1.0, p.panel);
                paint_fill(painter, Rect::from_min_max(chip.min, pos2(chip.right(), chip.top() + 5.0)), fill);
                paint_fill(painter, Rect::from_min_max(pos2(chip.left(), chip.bottom() - 5.0), chip.max), fill);
                paint_fill(painter, Rect::from_min_max(chip.min, pos2(chip.left() + 5.0, chip.bottom())), fill);
                paint_fill(painter, Rect::from_min_max(pos2(chip.right() - 5.0, chip.top()), chip.max), fill);
            }
            ColourTarget::Fill => paint_fill(painter, chip, fill),
        }
        if !enabled {
            painter.line_segment([chip.left_bottom(), chip.right_top()], Stroke::new(2.0, Color32::RED));
        }
        let highlighted = st.target == target;
        painter.rect_stroke(chip, 1.0, Stroke::new(if highlighted { 2.0 } else { 1.0 }, if highlighted { p.accent } else { p.border }), StrokeKind::Outside);
        let name = if matches!(target, ColourTarget::Fill) { "Fill colour" } else { "Stroke colour" };
        if response.on_hover_text(name).clicked() {
            clicked = Some(target);
        }
    }
    if let Some(target) = clicked {
        st.target = target;
        st.chip_popup = if st.chip_popup == Some(target) { None } else { Some(target) };
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if mini_button(ui, "Default colours (black stroke, white fill)", |painter, r, c| {
            painter.rect_stroke(Rect::from_min_size(r.min, vec2(8.0, 8.0)), 0.0, Stroke::new(1.5, c), StrokeKind::Inside);
            painter.rect_filled(Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(8.0, 8.0)), 0.0, Color32::WHITE);
            painter.rect_stroke(Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(8.0, 8.0)), 0.0, Stroke::new(1.0, c), StrokeKind::Inside);
        }) {
            events.push(ToolUiEvent::Command { id: "colour.defaults", params: json!({}) });
        }
        if mini_button(ui, "No colour (switches the selected colour off)", |painter, r, _| {
            painter.rect_stroke(r.shrink(2.0), 0.0, Stroke::new(1.0, Color32::GRAY), StrokeKind::Inside);
            painter.line_segment([r.left_bottom() + vec2(2.0, -2.0), r.right_top() + vec2(-2.0, 2.0)], Stroke::new(1.5, Color32::RED));
        }) {
            let key = if st.target == ColourTarget::Fill { "fill_enabled" } else { "stroke_enabled" };
            events.push(ToolUiEvent::options(json!({ key: false })));
        }
        if mini_button(ui, "Swap fill and stroke colours", |painter, r, c| {
            let s = Stroke::new(1.5, c);
            painter.line_segment([r.left_top() + vec2(3.0, 5.0), r.right_top() + vec2(-3.0, 5.0)], s);
            painter.line_segment([r.right_top() + vec2(-6.0, 2.0), r.right_top() + vec2(-3.0, 5.0)], s);
            painter.line_segment([r.left_bottom() + vec2(3.0, -5.0), r.right_bottom() + vec2(-3.0, -5.0)], s);
            painter.line_segment([r.left_bottom() + vec2(3.0, -5.0), r.left_bottom() + vec2(6.0, -2.0)], s);
        }) {
            events.push(ToolUiEvent::Command { id: "colour.swap", params: json!({}) });
        }
    });
    if let Some(target) = st.chip_popup {
        chip_popup(ui, target, swatches, st, events);
    }
}

fn mini_button(ui: &mut Ui, tip: &str, draw: impl FnOnce(&egui::Painter, Rect, Color32)) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    let p = current_palette(ui.ctx());
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.grid_line_strong);
    }
    draw(ui.painter(), rect, p.text);
    response.on_hover_text(tip).clicked()
}

/// The swatch popup of a colour chip.
fn chip_popup(ui: &Ui, target: ColourTarget, swatches: &[FillStyle], st: &mut ToolUi, events: &mut Vec<ToolUiEvent>) {
    let ctx = ui.ctx().clone();
    let anchor = ui.min_rect().right_top() + vec2(4.0, 40.0);
    let mut picked: Option<usize> = None;
    let mut none = false;
    let area = egui::Area::new(Id::new("chip-popup")).order(egui::Order::Foreground).fixed_pos(anchor).show(&ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_max_width(180.0);
            ui.label(if target == ColourTarget::Fill { "Fill colour" } else { "Stroke colour" });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                for (i, fill) in swatches.iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::click());
                    paint_fill(ui.painter(), rect, fill);
                    if response.clicked() {
                        picked = Some(i);
                    }
                }
            });
            if ui.small_button("No colour").clicked() {
                none = true;
            }
        });
    });
    if let Some(index) = picked {
        events.push(ToolUiEvent::Command { id: "swatches.use", params: json!({ "index": index, "stroke": target == ColourTarget::Stroke }) });
        st.chip_popup = None;
    } else if none {
        let key = if target == ColourTarget::Fill { "fill_enabled" } else { "stroke_enabled" };
        events.push(ToolUiEvent::options(json!({ key: false })));
        st.chip_popup = None;
    } else if ctx.input(|i| i.pointer.any_pressed())
        && !ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p: Pos2| area.response.rect.contains(p) || p.x < anchor.x)
    {
        st.chip_popup = None;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Option switches
// ---------------------------------------------------------------------------------------------------------------

fn switches(ui: &mut Ui, s: &ToolSettings, events: &mut Vec<ToolUiEvent>) {
    ui.label(egui::RichText::new("Options").small().weak());
    let toggles: [(&str, bool, &str, fn(&egui::Painter, Rect, Color32)); 3] = [
        ("Object Drawing", s.object_drawing, "object_drawing", |painter, r, c| {
            painter.rect_stroke(r.shrink(3.0), 3.0, Stroke::new(1.5, c), StrokeKind::Middle);
            painter.circle_filled(r.center(), 2.0, c);
        }),
        ("Snap to Objects", s.snap.objects, "objects", |painter, r, c| {
            let s = Stroke::new(2.0, c);
            painter.line_segment([r.center_top() + vec2(-5.0, 3.0), r.center_top() + vec2(-5.0, 9.0)], s);
            painter.line_segment([r.center_top() + vec2(5.0, 3.0), r.center_top() + vec2(5.0, 9.0)], s);
            painter.add(egui::Shape::line(
                vec![
                    pos2(r.center().x - 5.0, r.top() + 9.0),
                    pos2(r.center().x - 5.0, r.bottom() - 5.0),
                    pos2(r.center().x + 5.0, r.bottom() - 5.0),
                    pos2(r.center().x + 5.0, r.top() + 9.0),
                ],
                s,
            ));
        }),
        ("Snap to Pixels", s.snap.pixels, "pixels", |painter, r, c| {
            let s = Stroke::new(1.0, c);
            for i in 1..4_u8 {
                let t = f32::from(i) / 4.0;
                painter.line_segment(
                    [pos2(r.left() + 3.0 + (r.width() - 6.0) * t, r.top() + 3.0), pos2(r.left() + 3.0 + (r.width() - 6.0) * t, r.bottom() - 3.0)],
                    s,
                );
                painter.line_segment(
                    [pos2(r.left() + 3.0, r.top() + 3.0 + (r.height() - 6.0) * t), pos2(r.right() - 3.0, r.top() + 3.0 + (r.height() - 6.0) * t)],
                    s,
                );
            }
        }),
    ];
    ui.horizontal_wrapped(|ui| {
        for (tip, on, key, draw) in toggles {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            let p = current_palette(ui.ctx());
            if on {
                ui.painter().rect(rect, 3.0, p.accent_soft, Stroke::new(1.0, p.accent), StrokeKind::Inside);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 3.0, p.grid_line_strong);
            }
            draw(ui.painter(), rect, p.text);
            if response.on_hover_text(tip).clicked() {
                let patch = if key == "object_drawing" { json!({ key: !on }) } else { json!({ "snap": { key: !on } }) };
                events.push(ToolUiEvent::options(patch));
            }
        }
    });
}
