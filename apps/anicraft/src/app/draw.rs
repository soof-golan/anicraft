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
use crate::toolui;
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
        self.sync_modes();

        let mut events: Vec<UiEvent> = Vec::new();
        let menu_view = self.menu_view();
        egui::Panel::top("menu").show(ui, |ui| {
            events.extend(menus::menu_bar(ui, &menu_view).into_iter().map(UiEvent::Menu));
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.dock(ui, &mut events));
        events.extend(self.dialogs(&ctx));
        events.extend(self.scene_panel(&ctx));
        events.extend(self.parenting_view(&ctx));

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
        let closed_panels = [
            PanelId::Tools,
            PanelId::Stage,
            PanelId::Timeline,
            PanelId::Properties,
            PanelId::Library,
            PanelId::Custom(toolui::COLOUR_PANEL),
            PanelId::Custom(toolui::SWATCHES_PANEL),
        ]
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
            scene_panel: self.ui.scene_panel,
            parenting_view: self.ui.parenting_view,
            edit_multiple: self.ui.timeline.edit_multiple,
            auto_keyframe: self.ui.timeline.auto_keyframe,
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let doc = self.host.session().doc();
        let state = &doc.editor.state;
        let frame = state.frame.saturating_add(1);
        let total = self.frame_count();
        let text = if self.ui.status.is_empty() { "Ready".to_owned() } else { self.ui.status.clone() };
        let scope = match &state.timeline {
            anim_scene::TimelineId::Scene(i) => {
                doc.document().scenes.get(*i).map_or_else(|| "Scene".to_owned(), |s| anicraft_xfl::display_name(&s.name).into_owned())
            }
            anim_scene::TimelineId::Symbol(name) => format!("symbol {}", anicraft_xfl::display_name(name)),
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
        let previews = self.ui.timeline.preview != anim_ui::timeline::PreviewMode::Off;
        let model =
            view::timeline_model(doc.document(), &state.timeline, state.frame, self.ui.onion, self.ui.playback.is_some(), &self.ui.collapsed_layers, previews);
        let transport = TransportState {
            playing: self.ui.playback.is_some(),
            looping: self.ui.looping,
            fps: num::f64_to_f32(doc.document().frame_rate),
            frame: state.frame,
            frame_count: model.frame_count,
        };
        let thumbs: HashMap<String, egui::TextureId> = self.ui.thumbs.iter().map(|(k, v)| (k.clone(), v.id())).collect();
        let mut rows = view::library_rows(doc.document(), &self.ui.collapsed_folders, &self.ui.library_filter, self.ui.library_selected.as_deref(), &thumbs);
        if let Some(name) = self.ui.library_selected.as_deref() {
            rows.model.preview = crate::preview::model(doc.document(), name, &self.ui.preview, doc.document().frame_rate, std::time::Instant::now());
        }
        let props = crate::props::describe(doc);
        let tool_id = state.active_tool.toolbar_id().to_owned();
        let timeline_name = |id: &anim_scene::TimelineId| match id {
            anim_scene::TimelineId::Scene(i) => doc.document().scenes.get(*i).map_or_else(String::new, |s| anicraft_xfl::display_name(&s.name).into_owned()),
            anim_scene::TimelineId::Symbol(n) => anicraft_xfl::display_name(n).into_owned(),
        };
        // The edit bar: the chain of timelines entered, outermost first, and the scenes to switch between.
        let crumbs: Vec<String> =
            state.timeline_stack.iter().map(|(id, ..)| timeline_name(id)).chain(std::iter::once(timeline_name(&state.timeline))).collect();
        let scene_names: Vec<String> = doc.document().scenes.iter().map(|s| anicraft_xfl::display_name(&s.name).into_owned()).collect();
        let root_scene =
            if let anim_scene::TimelineId::Scene(i) = state.timeline_stack.first().map_or(&state.timeline, |(id, ..)| id) { Some(*i) } else { None };
        let editing_in_place = state.in_place.is_some();
        let to_stage = state.in_place.unwrap_or(anim_core::Affine::IDENTITY);
        let bg = doc.document().background_color;
        let selection = state.selection.items.clone();
        let rects = view::selection_rects(doc.document(), &state.timeline, &selection);
        let accent = self.ui.theme.palette().accent;
        let tool = state.active_tool;
        let settings = state.settings.clone();
        let swatches = anicraft_tools::swatches::read(doc.document());
        let bitmaps: Vec<String> = doc.document().library.bitmaps().map(|b| b.name.clone()).collect();

        let st = &mut self.ui;
        st.stage.grid_size = num::f64_to_f32(settings.snap.grid_size.clamp(1.0, 1000.0));
        st.stage.doc_size = vec2(num::f64_to_f32(doc.document().width), num::f64_to_f32(doc.document().height));
        st.stage.stage_color = egui::Color32::from_rgb(bg.r, bg.g, bg.b);
        st.stage.cursor = st.tool_cursor;
        let mut overlays = st.tool_overlay.clone();
        // The selection tool draws its own box; other tools show where the selection is.
        if overlays.is_empty() {
            overlays.extend(rects.iter().map(|r| view::selection_box(*r, accent, to_stage)));
        }
        st.stage.overlays = overlays;
        let texture = self.stage_tex.as_ref();
        let panel_rects = &mut st.panel_rects;
        let output = dock::show_dock(ui, egui::Id::new("anicraft-dock"), &st.layout, toolui::panel_title, |ui, panel| {
            panel_rects.insert(toolui::panel_title(*panel).to_lowercase(), ui.available_rect_before_wrap());
            match panel {
                PanelId::Tools => {
                    for e in toolui::toolbox(ui, &st.tools, &tool_id, &settings, &swatches, &mut st.toolui) {
                        events.push(UiEvent::ToolUi(e));
                    }
                }
                PanelId::Stage => {
                    ui.vertical(|ui| {
                        edit_bar(ui, &crumbs, &scene_names, root_scene, editing_in_place, events);
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
                    egui::CollapsingHeader::new("Tool").default_open(true).show(ui, |ui| {
                        for e in toolui::tool_options(ui, tool, &settings) {
                            events.push(UiEvent::ToolUi(e));
                        }
                    });
                    for e in anim_ui::property_grid(ui, &props) {
                        events.push(UiEvent::Property(e));
                    }
                }
                PanelId::Custom(toolui::COLOUR_PANEL) => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for e in toolui::colour_panel(ui, &settings, &bitmaps, &mut st.toolui) {
                            events.push(UiEvent::ToolUi(e));
                        }
                    });
                }
                PanelId::Custom(toolui::SWATCHES_PANEL) => {
                    for e in toolui::swatches_panel(ui, &swatches, &settings, &st.toolui) {
                        events.push(UiEvent::ToolUi(e));
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
            Dialog::RenameScene { .. } => "Rename Scene",
            Dialog::LayerProperties(_) => "Layer Properties",
            Dialog::FrameProperties(_) => "Frame Properties",
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
            Dialog::LayerProperties(p) => {
                egui::Grid::new("layer-props").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut p.name);
                    ui.end_row();
                    ui.label("Type");
                    ui.horizontal(|ui| {
                        for (i, label) in ["Normal", "Guide", "Mask", "Folder"].into_iter().enumerate() {
                            ui.radio_value(&mut p.kind, i, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Outline color");
                    ui.color_edit_button_srgb(&mut p.color);
                    ui.end_row();
                    ui.label("Layer height");
                    ui.horizontal(|ui| {
                        for (h, label) in [(1_u8, "100%"), (2, "200%"), (3, "300%")] {
                            ui.radio_value(&mut p.height, h, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Show");
                    ui.checkbox(&mut p.visible, "");
                    ui.end_row();
                    ui.label("Lock");
                    ui.checkbox(&mut p.locked, "");
                    ui.end_row();
                    ui.label("Outlines");
                    ui.checkbox(&mut p.outline, "");
                    ui.end_row();
                });
                ok_cancel(ui, &mut confirmed, &mut keep);
            }
            Dialog::FrameProperties(p) => {
                egui::Grid::new("frame-props").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut p.label);
                    ui.end_row();
                    ui.label("Type");
                    ui.horizontal(|ui| {
                        for (i, label) in ["Name", "Comment", "Anchor"].into_iter().enumerate() {
                            ui.radio_value(&mut p.label_kind, i, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Tween");
                    ui.label(p.tween.as_str());
                    ui.end_row();
                    ui.label("Ease");
                    ui.add_enabled(p.tween != "None", egui::Slider::new(&mut p.ease, -100..=100));
                    ui.end_row();
                    ui.label("Sound");
                    egui::ComboBox::from_id_salt("frame-sound").selected_text(if p.sound.is_empty() { "None" } else { p.sound.as_str() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut p.sound, String::new(), "None");
                        for name in p.sounds.clone() {
                            ui.selectable_value(&mut p.sound, name.clone(), name);
                        }
                    });
                    ui.end_row();
                    ui.label("Sync");
                    ui.horizontal(|ui| {
                        for (i, label) in ["Event", "Start", "Stop", "Stream"].into_iter().enumerate() {
                            ui.radio_value(&mut p.sync, i, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Repeat");
                    ui.horizontal(|ui| {
                        ui.add_enabled(!p.forever, egui::DragValue::new(&mut p.repeat).range(1..=10_000));
                        ui.checkbox(&mut p.forever, "Loop");
                    });
                    ui.end_row();
                });
                ui.label("Script");
                ui.add(egui::TextEdit::multiline(&mut p.script).code_editor().desired_rows(5).desired_width(360.0));
                ok_cancel(ui, &mut confirmed, &mut keep);
            }
            Dialog::RenameLayer { name, .. } | Dialog::RenameItem { name, .. } | Dialog::RenameScene { name, .. } => {
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

impl AniApp {
    /// The Scene panel (Window > Scene, Shift+F2): the scenes in playback order, to switch between, add, duplicate,
    /// delete, rename and reorder.
    fn scene_panel(&mut self, ctx: &Context) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if !self.ui.scene_panel {
            return out;
        }
        let doc = self.host.session().doc();
        let names: Vec<String> = doc.document().scenes.iter().map(|s| anicraft_xfl::display_name(&s.name).into_owned()).collect();
        let root = doc.editor.state.timeline_stack.first().map_or(&doc.editor.state.timeline, |entry| &entry.0);
        let open_scene = if let anim_scene::TimelineId::Scene(i) = root { Some(*i) } else { None };
        let selected = open_scene.unwrap_or(0);
        let mut open = true;
        let command = |id: &'static str, params: serde_json::Value| UiEvent::Menu(MenuAction::Command { id, params });
        egui::Window::new("Scene").open(&mut open).default_width(220.0).resizable(true).show(ctx, |ui| {
            for (i, name) in names.iter().enumerate() {
                let response = ui.selectable_label(Some(i) == open_scene, crate::text(name));
                if response.double_clicked() {
                    out.push(UiEvent::Menu(MenuAction::Dialog(Dialog::RenameScene { index: i, name: name.clone() })));
                } else if response.clicked() {
                    out.push(command("editor.goto", serde_json::json!({ "scene": i, "frame": 0 })));
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Add").on_hover_text("Add Scene").clicked() {
                    out.push(command("scene.add", serde_json::json!({ "name": format!("Scene {}", names.len().saturating_add(1)) })));
                }
                if ui.button("Duplicate").clicked() {
                    out.push(command("scene.duplicate", serde_json::json!({ "scene": selected })));
                }
                if ui.add_enabled(names.len() > 1, egui::Button::new("Delete")).clicked() {
                    out.push(command("scene.remove", serde_json::json!({ "scene": selected })));
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(selected > 0, egui::Button::new("Move Up")).clicked() {
                    out.push(command("scene.move", serde_json::json!({ "scene": selected, "to": selected.saturating_sub(1) })));
                }
                if ui.add_enabled(selected.saturating_add(1) < names.len(), egui::Button::new("Move Down")).clicked() {
                    out.push(command("scene.move", serde_json::json!({ "scene": selected, "to": selected.saturating_add(1) })));
                }
            });
        });
        self.ui.scene_panel = open;
        out
    }

    /// The Parenting view: every layer with the folder, mask or guide layer it hangs under, to change.
    fn parenting_view(&mut self, ctx: &Context) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if !self.ui.parenting_view {
            return out;
        }
        let doc = self.host.session().doc();
        let rows: Vec<(String, anim_scene::timeline::LayerKind, Option<usize>)> = doc
            .document()
            .timeline(&doc.editor.state.timeline)
            .map(|t| t.layers.iter().map(|l| (anicraft_xfl::display_name(&l.name).into_owned(), l.kind, l.parent_layer_index)).collect())
            .unwrap_or_default();
        let mut open = true;
        egui::Window::new("Parenting View").open(&mut open).default_width(320.0).resizable(true).show(ctx, |ui| {
            egui::Grid::new("parenting").num_columns(2).striped(true).show(ui, |ui| {
                ui.label(RichText::new("Layer").strong());
                ui.label(RichText::new("Parent").strong());
                ui.end_row();
                for (index, (name, kind, parent)) in rows.iter().enumerate() {
                    ui.label(format!("{} {}", kind_glyph(*kind), crate::text(name)));
                    let current = parent.and_then(|p| rows.get(p)).map_or_else(|| "None".to_owned(), |(n, ..)| crate::text(n));
                    egui::ComboBox::from_id_salt(("parent", index)).selected_text(current).show_ui(ui, |ui| {
                        if ui.selectable_label(parent.is_none(), "None").clicked() {
                            out.push(UiEvent::Menu(MenuAction::Command { id: "layer.set_parent", params: serde_json::json!({ "layer": index }) }));
                        }
                        for (candidate, (cname, ckind, _)) in rows.iter().enumerate().take(index) {
                            if matches!(
                                ckind,
                                anim_scene::timeline::LayerKind::Folder | anim_scene::timeline::LayerKind::Mask | anim_scene::timeline::LayerKind::Guide
                            ) && ui.selectable_label(*parent == Some(candidate), crate::text(cname)).clicked()
                            {
                                out.push(UiEvent::Menu(MenuAction::Command {
                                    id: "layer.set_parent",
                                    params: serde_json::json!({ "layer": index, "parent": candidate }),
                                }));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
        self.ui.parenting_view = open;
        out
    }
}

/// A one-character marker of a layer's type.
const fn kind_glyph(kind: anim_scene::timeline::LayerKind) -> &'static str {
    use anim_scene::timeline::LayerKind as K;
    match kind {
        K::Folder => "[F]",
        K::Mask => "[M]",
        K::Guide => "[G]",
        K::Camera => "[C]",
        K::Normal | K::Guided | K::Masked => "   ",
    }
}

/// The edit bar above the stage: the scene menu and the breadcrumb of the timelines entered, each level a button
/// that leaves the levels below it. A symbol edited in place is marked.
fn edit_bar(ui: &mut egui::Ui, crumbs: &[String], scenes: &[String], root_scene: Option<usize>, in_place: bool, events: &mut Vec<UiEvent>) {
    ui.horizontal(|ui| {
        if crumbs.len() > 1 && ui.button("<").on_hover_text("Back one level").clicked() {
            events.push(UiEvent::Menu(MenuAction::Command { id: "edit.exit_to", params: serde_json::json!({ "depth": crumbs.len().saturating_sub(2) }) }));
        }
        let selected = root_scene.and_then(|i| scenes.get(i)).map_or("Scene", String::as_str);
        egui::ComboBox::from_id_salt("edit-bar-scenes").selected_text(crate::text(selected)).width(110.0).show_ui(ui, |ui| {
            for (i, name) in scenes.iter().enumerate() {
                if ui.selectable_label(root_scene == Some(i), crate::text(name)).clicked() {
                    events.push(UiEvent::Menu(MenuAction::Command { id: "editor.goto", params: serde_json::json!({ "scene": i, "frame": 0 }) }));
                }
            }
        });
        // The first crumb is the scene (or symbol) the chain started in; the scene menu stands for it.
        let last = crumbs.len().saturating_sub(1);
        for (depth, name) in crumbs.iter().enumerate().skip(1) {
            ui.label(RichText::new(">").weak());
            if depth == last {
                ui.label(RichText::new(crate::text(name)).strong());
            } else if ui.link(crate::text(name)).clicked() {
                events.push(UiEvent::Menu(MenuAction::Command { id: "edit.exit_to", params: serde_json::json!({ "depth": depth }) }));
            }
        }
        if crumbs.len() == 1 && root_scene.is_none() {
            ui.label(RichText::new(crate::text(crumbs.first().map_or("", String::as_str))).strong());
        }
        if in_place {
            ui.label(RichText::new("editing in place").weak());
        }
    });
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
