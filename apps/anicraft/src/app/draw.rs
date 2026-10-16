//! Drawing a frame: menu, panels in the dock, status bar and dialogs. Every widget reports events; nothing here
//! changes the document.

use std::collections::HashMap;

use anim_core::num;
use anim_ui::dock::{self, docking::Layout};
use anim_ui::{PanelId, TransportState};
use egui::{Context, RichText, vec2};

use super::AniApp;
use super::actions::UiEvent;
use crate::menus::{self, MenuAction, MenuView};
use crate::state::Dialog;
use crate::view;

impl eframe::App for AniApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

impl AniApp {
    /// One frame of the whole window. Separate from [`eframe::App::ui`] so tests can run it without a window.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frames_drawn = self.frames_drawn.saturating_add(1);
        self.fonts = true;
        self.receive(&ctx);
        self.keyboard(&ctx);
        self.ui.theme.apply(&ctx);

        let mut events: Vec<UiEvent> = Vec::new();
        let menu_view = self.menu_view();
        egui::Panel::top("menu").show(ui, |ui| {
            events.extend(menus::menu_bar(ui, &menu_view).into_iter().map(UiEvent::Menu));
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.dock(ui, &mut events));
        events.extend(self.dialogs(&ctx));

        for event in events {
            self.handle(&ctx, event);
        }
        self.sync_worker(&ctx);
        self.drive_shot(&ctx);
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn menu_view(&self) -> MenuView {
        let session = self.host.session();
        let doc = session.doc();
        let enabled = menus::menu_commands().iter().copied().filter(|id| session.is_enabled(id)).collect();
        let closed_panels = [PanelId::Tools, PanelId::Stage, PanelId::Timeline, PanelId::Properties, PanelId::Library]
            .into_iter()
            .filter(|p| !self.ui.layout.contains(p))
            .collect();
        MenuView {
            enabled,
            can_undo: session.history().undo_label().map(str::to_owned),
            can_redo: session.history().redo_label().map(str::to_owned),
            playing: self.ui.playback.is_some(),
            looping: self.ui.looping,
            onion: self.ui.onion.enabled,
            onion_outlines: self.ui.onion.outlines,
            rulers: self.ui.stage.show_rulers,
            grid: self.ui.stage.show_grid,
            guides: self.ui.stage.show_guides,
            has_selection: !doc.editor.state.selection.is_empty(),
            closed_panels,
            recent: self.recent_paths(),
            dirty: session.is_dirty(),
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let doc = self.host.session().doc();
        let state = &doc.editor.state;
        let frame = state.frame.saturating_add(1);
        let total = self.frame_count();
        let text = if self.ui.status.is_empty() { "Ready".to_owned() } else { self.ui.status.clone() };
        let scope = match &state.timeline {
            anim_scene::TimelineId::Scene(i) => doc.document().scenes.get(*i).map_or_else(|| "Scene".to_owned(), |s| s.name.clone()),
            anim_scene::TimelineId::Symbol(name) => format!("symbol {name}"),
        };
        let zoom = (self.ui.stage.view.zoom * 100.0).round();
        let selected = state.selection.items.len();
        let took = self.sync.last_took.as_secs_f64() * 1000.0;
        let fps = doc.document().frame_rate;
        ui.horizontal(|ui| {
            ui.label(crate::text(&text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} | frame {frame}/{total} | {fps} fps | {zoom:.0}% | {selected} selected | render {took:.0} ms",
                        crate::text(&scope)
                    ))
                    .weak(),
                );
                if let Some(export) = &self.ui.export {
                    ui.add(egui::ProgressBar::new(export.fraction).desired_width(140.0).text(crate::text(&export.label)));
                }
            });
        });
    }

    fn dock(&mut self, ui: &mut egui::Ui, events: &mut Vec<UiEvent>) {
        let session = self.host.session();
        let doc = session.doc();
        let state = &doc.editor.state;
        let model = view::timeline_model(doc.document(), &state.timeline, state.frame, self.ui.onion, self.ui.playback.is_some(), &self.ui.collapsed_layers);
        let transport = TransportState {
            playing: self.ui.playback.is_some(),
            looping: self.ui.looping,
            fps: num::f64_to_f32(doc.document().frame_rate),
            frame: state.frame,
            frame_count: model.frame_count,
        };
        let thumbs: HashMap<String, egui::TextureId> = self.ui.thumbs.iter().map(|(k, v)| (k.clone(), v.id())).collect();
        let rows = view::library_rows(doc.document(), &self.ui.collapsed_folders, &self.ui.library_filter, self.ui.library_selected.as_deref(), &thumbs);
        let props = crate::props::describe(doc);
        let tool_id = state.active_tool.toolbar_id().to_owned();
        let in_symbol = matches!(state.timeline, anim_scene::TimelineId::Symbol(_));
        let scope_name = match &state.timeline {
            anim_scene::TimelineId::Scene(i) => doc.document().scenes.get(*i).map_or_else(String::new, |s| s.name.clone()),
            anim_scene::TimelineId::Symbol(n) => n.clone(),
        };
        let bg = doc.document().background_color;
        let selection = state.selection.items.clone();
        let rects = view::selection_rects(doc.document(), &state.timeline, &selection);
        let accent = self.ui.theme.palette().accent;

        let st = &mut self.ui;
        st.stage.doc_size = vec2(num::f64_to_f32(doc.document().width), num::f64_to_f32(doc.document().height));
        st.stage.stage_color = egui::Color32::from_rgb(bg.r, bg.g, bg.b);
        st.stage.cursor = st.tool_cursor;
        let mut overlays = st.tool_overlay.clone();
        // The selection tool draws its own box; other tools show where the selection is.
        if overlays.is_empty() {
            overlays.extend(rects.iter().map(|r| view::selection_box(*r, accent)));
        }
        st.stage.overlays = overlays;
        let texture = self.stage_tex.as_ref();
        let panel_rects = &mut st.panel_rects;
        let output = dock::show_dock(ui, egui::Id::new("anicraft-dock"), &st.layout, PanelId::title, |ui, panel| {
            panel_rects.insert(panel.title().to_lowercase(), ui.available_rect_before_wrap());
            match panel {
                PanelId::Tools => {
                    for anim_ui::ToolbarEvent::Select(id) in anim_ui::toolbar(ui, &st.tools, &tool_id) {
                        events.push(UiEvent::Tool(id));
                    }
                }
                PanelId::Stage => {
                    ui.vertical(|ui| {
                        if in_symbol {
                            ui.horizontal(|ui| {
                                if ui.button("< Back").clicked() {
                                    events.push(UiEvent::Menu(MenuAction::Command { id: "edit.exit", params: serde_json::json!({}) }));
                                }
                                ui.label(RichText::new(crate::text(&scope_name)).strong());
                            });
                        }
                        let bar = 34.0;
                        let rest = (ui.available_height() - bar).max(40.0);
                        ui.allocate_ui(vec2(ui.available_width(), rest), |ui| {
                            events.extend(anim_ui::stage_view(ui, &st.stage, texture).into_iter().map(UiEvent::Stage));
                        });
                        events.extend(anim_ui::transport_bar(ui, &transport).into_iter().map(UiEvent::Transport));
                    });
                }
                PanelId::Timeline => {
                    for e in anim_ui::timeline(ui, &model, &st.timeline) {
                        events.push(UiEvent::Timeline(e));
                    }
                }
                PanelId::Properties => {
                    for e in anim_ui::property_grid(ui, &props) {
                        events.push(UiEvent::Property(e));
                    }
                }
                PanelId::Library => {
                    for e in anim_ui::library_panel(ui, &rows.model) {
                        events.push(UiEvent::Library(e));
                    }
                }
                PanelId::Output | PanelId::Custom(_) => {}
            }
        });
        let actions = output.actions;
        let layout: &mut Layout<PanelId> = &mut st.layout;
        dock::apply_actions(layout, actions);
    }

    fn dialogs(&mut self, ctx: &Context) -> Vec<UiEvent> {
        let mut out = Vec::new();
        let Some(mut dialog) = self.ui.dialog.take() else { return out };
        let mut keep = true;
        let mut confirmed = false;
        let title = match &dialog {
            Dialog::ConvertToSymbol { .. } => "Convert to Symbol",
            Dialog::NewSymbol { .. } => "Create New Symbol",
            Dialog::RenameLayer { .. } => "Layer Name",
            Dialog::RenameItem { .. } => "Rename Item",
            Dialog::DocumentSettings => "Document Settings",
            Dialog::Message { title, .. } => title.as_str(),
            Dialog::Discard(_) => "Unsaved changes",
            Dialog::About => "AniCraft",
        }
        .to_owned();
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| match &mut dialog {
            Dialog::ConvertToSymbol { name, kind } | Dialog::NewSymbol { name, kind } => {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let response = ui.text_edit_singleline(name);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirmed = true;
                    }
                    response.request_focus();
                });
                ui.horizontal(|ui| {
                    ui.label("Type");
                    for (i, label) in ["Graphic", "Movie Clip", "Button"].into_iter().enumerate() {
                        ui.radio_value(kind, i, label);
                    }
                });
                ok_cancel(ui, &mut confirmed, &mut keep);
            }
            Dialog::RenameLayer { name, .. } | Dialog::RenameItem { name, .. } => {
                let response = ui.text_edit_singleline(name);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }
                response.request_focus();
                ok_cancel(ui, &mut confirmed, &mut keep);
            }
            Dialog::DocumentSettings => {
                let doc = self.host.session().doc().document();
                let (mut w, mut h, mut fps) = (doc.width, doc.height, doc.frame_rate);
                let mut bg = [doc.background_color.r, doc.background_color.g, doc.background_color.b];
                let mut changed = false;
                egui::Grid::new("doc-settings").num_columns(2).show(ui, |ui| {
                    ui.label("Width");
                    changed |= ui.add(egui::DragValue::new(&mut w).range(1.0..=16000.0).suffix(" px")).changed();
                    ui.end_row();
                    ui.label("Height");
                    changed |= ui.add(egui::DragValue::new(&mut h).range(1.0..=16000.0).suffix(" px")).changed();
                    ui.end_row();
                    ui.label("Frame rate");
                    changed |= ui.add(egui::DragValue::new(&mut fps).range(1.0..=240.0).suffix(" fps")).changed();
                    ui.end_row();
                    ui.label("Stage colour");
                    changed |= ui.color_edit_button_srgb(&mut bg).changed();
                    ui.end_row();
                });
                if changed {
                    out.push(UiEvent::Menu(MenuAction::Command {
                        id: "document.set_stage",
                        params: serde_json::json!({ "width": w, "height": h, "frame_rate": fps, "background": format!("#{:02x}{:02x}{:02x}", bg[0], bg[1], bg[2]) }),
                    }));
                }
                if ui.button("Close").clicked() {
                    keep = false;
                }
            }
            Dialog::Message { text, .. } => {
                ui.label(crate::text(text));
                if ui.button("OK").clicked() {
                    keep = false;
                }
            }
            Dialog::Discard(_) => {
                ui.label("The document has changes that were not saved. Discard them?");
                ui.horizontal(|ui| {
                    if ui.button("Discard").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            }
            Dialog::About => {
                ui.label(RichText::new("AniCraft").heading());
                ui.label("A 2D animation editor for Adobe Animate documents (.fla / XFL).");
                ui.label("Tools: V select, A subselect, Q transform, N line, R rectangle, O oval, P pen, Y pencil, B brush, K bucket, I eyedropper, E eraser, T text, H hand, Z zoom");
                ui.label("F5 frame, F6 keyframe, F7 blank keyframe, F8 convert to symbol, Enter play, Ctrl+Z undo");
                ui.label("The running editor is also an MCP server: start it with --mcp-stdio, or --control-port for the anim-mcp bridge.");
                if ui.button("Close").clicked() {
                    keep = false;
                }
            }
        });
        if confirmed {
            self.dialog_ok(dialog);
        } else if keep {
            self.ui.dialog = Some(dialog);
        }
        out
    }
}

fn ok_cancel(ui: &mut egui::Ui, confirmed: &mut bool, keep: &mut bool) {
    ui.horizontal(|ui| {
        if ui.button("OK").clicked() {
            *confirmed = true;
        }
        if ui.button("Cancel").clicked() {
            *keep = false;
        }
    });
}
