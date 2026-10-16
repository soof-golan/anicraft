//! The Swatches panel: the document's palette as a grid, with add, delete, load and save.

use anicraft_tools::ToolSettings;
use anicraft_tools::swatches::{TableFormat, default_swatches};
use anim_shape::FillStyle;
use anim_ui::theme::current_palette;
use egui::{Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::json;

use super::toolbox::paint_fill;
use super::{ColourTarget, ToolUi, ToolUiEvent};

const CELL: f32 = 18.0;

/// Draws the palette. A click makes a swatch the colour the Colour panel is editing; shift or the context menu
/// uses it for the other colour.
pub fn swatches_panel(ui: &mut Ui, swatches: &[FillStyle], s: &ToolSettings, st: &ToolUi) -> Vec<ToolUiEvent> {
    let mut events = Vec::new();
    ui.horizontal(|ui| {
        if ui.button("+").on_hover_text("Add the current fill to the swatches").clicked() {
            events.push(ToolUiEvent::Command { id: "swatches.add", params: json!({}) });
        }
        ui.menu_button("Options", |ui| {
            if ui.button("Load Colors...").clicked() {
                events.push(ToolUiEvent::SwatchLoad);
                ui.close();
            }
            if ui.button("Save Colors (.aco)...").clicked() {
                events.push(ToolUiEvent::SwatchSave(TableFormat::Aco));
                ui.close();
            }
            if ui.button("Save Colors (.act)...").clicked() {
                events.push(ToolUiEvent::SwatchSave(TableFormat::Act));
                ui.close();
            }
            ui.separator();
            if ui.button("Load Default Colors").clicked() {
                events.push(ToolUiEvent::Command { id: "swatches.set", params: json!({ "swatches": default_swatches() }) });
                ui.close();
            }
            if ui.button("Clear Colors").clicked() {
                events.push(ToolUiEvent::Command { id: "swatches.set", params: json!({ "swatches": [] }) });
                ui.close();
            }
        });
        ui.weak(format!("{} colours", swatches.len()));
    });
    ui.separator();
    let palette = current_palette(ui.ctx());
    let target_stroke = st.target == ColourTarget::Stroke;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(3.0);
        ui.horizontal_wrapped(|ui| {
            for (index, fill) in swatches.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
                paint_fill(ui.painter(), rect, fill);
                let current = if target_stroke { &s.stroke.solid().fill } else { &s.fill };
                let selected = current == fill;
                let outline = if selected { Stroke::new(2.0, palette.accent) } else { Stroke::new(1.0, palette.border) };
                ui.painter().rect_stroke(rect, 0.0, outline, StrokeKind::Outside);
                let shift = ui.input(|i| i.modifiers.shift);
                if response.clicked() {
                    events.push(ToolUiEvent::Command { id: "swatches.use", params: json!({ "index": index, "stroke": target_stroke != shift }) });
                }
                response.context_menu(|ui| {
                    if ui.button("Use as fill").clicked() {
                        events.push(ToolUiEvent::Command { id: "swatches.use", params: json!({ "index": index, "stroke": false }) });
                        ui.close();
                    }
                    if ui.button("Use as stroke").clicked() {
                        events.push(ToolUiEvent::Command { id: "swatches.use", params: json!({ "index": index, "stroke": true }) });
                        ui.close();
                    }
                    if ui.button("Delete swatch").clicked() {
                        events.push(ToolUiEvent::Command { id: "swatches.delete", params: json!({ "index": index }) });
                        ui.close();
                    }
                });
            }
        });
    });
    events
}
