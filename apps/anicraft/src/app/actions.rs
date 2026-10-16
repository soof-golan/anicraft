//! Events from the panels become commands on the session. This is the only place where UI input changes the document.

use std::path::PathBuf;
use std::time::Instant;

use anicraft_cli::mcp::AniDoc;
use anicraft_tools::{ElementRef, Selection, ToolId};
use anim_core::{Affine, Point, num};
use anim_scene::edit;
use anim_scene::timeline::LayerKind;
use anim_scene::{Element, LibraryItem, TimelineId};
use anim_ui::library::{LibraryAction, LibraryEvent};
use anim_ui::stage::{Guide, StageEvent};
use anim_ui::timeline::{DropPlacement, FrameAction, FrameSelection, LayerEvent, TimelineEvent};
use anim_ui::{PropertyEdit, TransportEvent};
use egui::Context;
use serde_json::{Value, json};

use super::AniApp;
use crate::menus::MenuAction;
use crate::state::{Dialog, ExportStatus, FrameProps, LayerProps, Playback};
use crate::worker::Job;

/// Everything a panel can report in one frame.
#[derive(Debug)]
pub enum UiEvent {
    Menu(MenuAction),
    Stage(StageEvent),
    Timeline(TimelineEvent),
    Library(LibraryEvent),
    Property(PropertyEdit),
    Tool(String),
    Transport(TransportEvent),
}

/// The scene model's layer type for the timeline widget's.
const fn scene_kind(kind: anim_ui::timeline::LayerKind) -> LayerKind {
    use anim_ui::timeline::LayerKind as K;
    match kind {
        K::Guide | K::Guided => LayerKind::Guide,
        K::Mask | K::Masked => LayerKind::Mask,
        K::Folder => LayerKind::Folder,
        K::Normal | K::Camera => LayerKind::Normal,
    }
}

/// The name `layer.set_kind` knows a layer type by.
const fn kind_name(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Guide => "guide",
        LayerKind::Mask => "mask",
        LayerKind::Folder => "folder",
        _ => "normal",
    }
}

/// A change to frames the Insert menu, the timeline's context menu and the function keys share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOp {
    InsertFrames,
    RemoveFrames,
    InsertKeyframe,
    InsertBlankKeyframe,
    ClearKeyframe,
    ClassicTween,
    ShapeTween,
    RemoveTween,
}

impl FrameOp {
    const fn label(self) -> &'static str {
        match self {
            Self::InsertFrames => "Insert Frames",
            Self::RemoveFrames => "Remove Frames",
            Self::InsertKeyframe => "Insert Keyframe",
            Self::InsertBlankKeyframe => "Insert Blank Keyframe",
            Self::ClearKeyframe => "Clear Keyframe",
            Self::ClassicTween => "Create Classic Tween",
            Self::ShapeTween => "Create Shape Tween",
            Self::RemoveTween => "Remove Tween",
        }
    }
}

impl AniApp {
    pub(super) fn handle(&mut self, ctx: &Context, event: UiEvent) {
        match event {
            UiEvent::Menu(action) => self.menu(ctx, action),
            UiEvent::Stage(event) => self.stage_event(event),
            UiEvent::Timeline(event) => self.timeline_event(event),
            UiEvent::Library(event) => self.library_event(event),
            UiEvent::Property(edit) => self.property_edit(&edit),
            UiEvent::Tool(id) => self.select_tool_by_id(&id),
            UiEvent::Transport(event) => self.transport_event(event),
        }
    }

    pub(crate) fn status(&mut self, text: impl Into<String>) {
        self.ui.status = text.into();
    }

    // ---- menus ------------------------------------------------------------------------------------------------

    fn menu(&mut self, ctx: &Context, action: MenuAction) {
        match action {
            MenuAction::Command { id, params } => self.menu_command(id, params),
            MenuAction::New => self.new_document(),
            MenuAction::Open => self.open_dialog(),
            MenuAction::OpenRecent(path) => self.open_path(path),
            MenuAction::Save => self.save(false),
            MenuAction::SaveAs => self.save(true),
            MenuAction::Import { to_stage } => self.import_dialog(to_stage),
            MenuAction::Export(kind) => self.export_dialog(kind),
            MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            MenuAction::Cut => {
                self.copy_selection();
                self.run("edit.delete", &json!({}));
            }
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Paste => self.paste(),
            MenuAction::ZoomIn => self.zoom(1.25),
            MenuAction::ZoomOut => self.zoom(0.8),
            MenuAction::ZoomFit => self.ui.stage.fit_request = true,
            MenuAction::Zoom100 => {
                let canvas = self.stage_canvas();
                self.ui.stage.view = anim_ui::StageView::actual_size(canvas.size(), self.ui.stage.doc_size);
            }
            MenuAction::ToggleOnion => self.ui.onion.enabled = !self.ui.onion.enabled,
            MenuAction::ToggleOnionOutlines => self.ui.onion.outlines = !self.ui.onion.outlines,
            MenuAction::ToggleRulers => self.ui.stage.show_rulers = !self.ui.stage.show_rulers,
            MenuAction::ToggleGrid => self.ui.stage.show_grid = !self.ui.stage.show_grid,
            MenuAction::ToggleGuides => self.ui.stage.show_guides = !self.ui.stage.show_guides,
            MenuAction::ToggleTheme => {
                self.ui.theme = self.ui.theme.toggled();
                self.ui.theme.apply(ctx);
            }
            MenuAction::OpenPanel(panel) => {
                if let Err(e) = anim_ui::dock::open_panel(&mut self.ui.layout, panel) {
                    self.status(format!("cannot open {}: {e}", panel.title()));
                }
            }
            MenuAction::Play => self.toggle_play(),
            MenuAction::Stop => {
                self.stop_playback();
                self.goto_frame(0);
            }
            MenuAction::ToggleLoop => self.ui.looping = !self.ui.looping,
            MenuAction::GoStart => self.goto_frame(0),
            MenuAction::GoEnd => {
                let last = self.frame_count().saturating_sub(1);
                self.goto_frame(last);
            }
            MenuAction::StepForward => self.step(1),
            MenuAction::StepBack => self.step(-1),
            MenuAction::Dialog(dialog) => self.ui.dialog = Some(dialog),
            MenuAction::Frame(action) => self.frame_action(action),
            MenuAction::AddLayer(kind) => self.add_layer(scene_kind(kind)),
            MenuAction::AddMotionGuide => {
                let layer = self.host.session().doc().editor.state.layer;
                self.run("layer.add_motion_guide", &json!({ "layer": layer }));
            }
            MenuAction::LayerProperties => {
                let layer = self.host.session().doc().editor.state.layer;
                self.open_layer_properties(layer);
            }
            MenuAction::EditInPlace => self.edit_selected_in_place(),
            MenuAction::EditSymbol => self.edit_symbol_of_selection(),
            MenuAction::ToggleScenePanel => self.ui.scene_panel = !self.ui.scene_panel,
            MenuAction::ToggleParentingView => self.ui.parenting_view = !self.ui.parenting_view,
            MenuAction::ToggleEditMultiple => self.ui.timeline.edit_multiple = !self.ui.timeline.edit_multiple,
            MenuAction::ToggleAutoKeyframe => self.ui.timeline.auto_keyframe = !self.ui.timeline.auto_keyframe,
        }
    }

    /// Edit Symbols (Ctrl+E): the selected instance's symbol, else the symbol selected in the library.
    fn edit_symbol_of_selection(&mut self) {
        let doc = self.host.session().doc();
        let from_stage = doc
            .editor
            .state
            .selection
            .items
            .first()
            .and_then(|r| anicraft_tools::resolve_ref(doc.document(), &doc.editor.state.timeline, r))
            .and_then(|r| if let Element::SymbolInstance(i) = r.element { Some(i.library_item_name.clone()) } else { None });
        match from_stage.or_else(|| self.ui.library_selected.clone()) {
            Some(name) => self.edit_symbol(&name),
            None => self.status("select a symbol instance or a library symbol first"),
        }
    }

    /// Menu items for the timeline map to frame operations that also work inside symbols; the rest are commands.
    fn menu_command(&mut self, id: &'static str, params: Value) {
        let op = match id {
            "timeline.insert_frames" => Some(FrameOp::InsertFrames),
            "timeline.insert_keyframe" => Some(FrameOp::InsertKeyframe),
            "timeline.insert_blank_keyframe" => Some(FrameOp::InsertBlankKeyframe),
            "timeline.create_classic_tween" => Some(FrameOp::ClassicTween),
            "timeline.create_shape_tween" => Some(FrameOp::ShapeTween),
            "timeline.remove_tween" => Some(FrameOp::RemoveTween),
            _ => None,
        };
        if let Some(op) = op {
            self.frame_op(op);
            return;
        }
        if id == "layer.add" {
            self.add_layer(LayerKind::Normal);
            return;
        }
        if id == "scene.add" {
            let before = self.host.session().doc().document().scenes.len();
            if self.run(id, &params) {
                // A scene that was just made is the one to work in.
                self.go_to_scene(before);
            }
            return;
        }
        self.run(id, &params);
    }

    /// Runs a registered command as one undo step and reports failures in the status bar.
    pub(crate) fn run(&mut self, id: &str, params: &Value) -> bool {
        let result = self.host.session_mut().execute(id, params);
        match result {
            Ok(_) => {
                if id.starts_with("edit.undo") || id.starts_with("edit.redo") {
                    self.ui.drag_base = None;
                }
                true
            }
            Err(e) => {
                self.status(format!("{id}: {e}"));
                false
            }
        }
    }

    // ---- stage ------------------------------------------------------------------------------------------------

    fn stage_event(&mut self, event: StageEvent) {
        match event {
            StageEvent::ViewChanged(view) => {
                self.ui.stage.view = view;
                self.ui.stage.fit_request = false;
                self.host.session_mut().doc_mut().editor.state.view.zoom = f64::from(view.zoom);
            }
            StageEvent::Pointer(pointer) => self.stage_pointer(&pointer),
            StageEvent::GuideAdded(guide) => self.ui.stage.guides.push(guide),
            StageEvent::GuideMoved { index, pos } => {
                if let Some(guide) = self.ui.stage.guides.get_mut(index) {
                    *guide = Guide { axis: guide.axis, pos };
                }
            }
            StageEvent::GuideRemoved { index } => {
                if index < self.ui.stage.guides.len() {
                    self.ui.stage.guides.remove(index);
                }
            }
            StageEvent::DropLibraryItem { id, pos } => self.drop_library_item(id, Point::new(f64::from(pos.x), f64::from(pos.y))),
        }
    }

    fn drop_library_item(&mut self, id: u64, at: Point) {
        let Some(name) = self.library_name(id) else { return };
        let (timeline, layer, frame) = {
            let s = &self.host.session().doc().editor.state;
            (s.timeline.clone(), s.layer, s.frame)
        };
        let result = self.host.session_mut().edit("Place from Library", None, |doc: &mut AniDoc| -> Result<usize, String> {
            let document = &mut doc.editor.document;
            match document.library.find(&name) {
                Some(LibraryItem::Symbol(_)) => {
                    edit::add_symbol_instance(document, &timeline, layer, frame, &name, Affine::translate(at.to_vec2())).map_err(|e| e.to_string())
                }
                Some(LibraryItem::Bitmap(b)) => {
                    let size = b.size.unwrap_or([0.0, 0.0]);
                    let m = Affine::translate((at.x - size[0] / 2.0, at.y - size[1] / 2.0));
                    edit::add_element(document, &timeline, layer, frame, Element::Bitmap(anim_scene::BitmapInstance::new(&name, m))).map_err(|e| e.to_string())
                }
                _ => Err(format!("{name} cannot be placed on the stage")),
            }
        });
        match result {
            Ok(index) => {
                let keyframe = self.keyframe_index(layer, frame);
                self.host.session_mut().doc_mut().editor.state.selection = Selection::one(ElementRef::top_level(layer, keyframe, index));
                self.status(format!("placed {name}"));
            }
            Err(e) => {
                self.host.session_mut().undo();
                self.status(e);
            }
        }
    }

    pub(crate) fn keyframe_index(&self, layer: usize, frame: u32) -> u32 {
        let doc = self.host.session().doc();
        doc.document().timeline(&doc.editor.state.timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map_or(frame, |k| k.index)
    }

    fn library_name(&self, id: u64) -> Option<String> {
        let doc = self.host.session().doc().document();
        let rows = crate::view::library_rows(
            doc,
            &self.ui.collapsed_folders,
            &self.ui.library_filter,
            self.ui.library_selected.as_deref(),
            &std::collections::HashMap::new(),
        );
        rows.names.get(num::u32_to_usize(u32::try_from(id).ok()?)).cloned()
    }

    pub(crate) fn stage_canvas(&self) -> egui::Rect {
        self.ui.panel_rects.get("stage").copied().unwrap_or_else(|| egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0)))
    }

    fn zoom(&mut self, factor: f32) {
        let canvas = self.stage_canvas();
        let target = (self.ui.stage.view.zoom * factor).clamp(0.02, 64.0);
        self.ui.stage.view = self.ui.stage.view.with_zoom_centered(egui::Rect::from_min_size(egui::Pos2::ZERO, canvas.size()), target);
    }

    // ---- tools ------------------------------------------------------------------------------------------------

    fn select_tool_by_id(&mut self, id: &str) {
        match ToolId::from_toolbar_id(id) {
            Some(tool) => {
                self.host.session_mut().doc_mut().editor.select_tool(tool);
                self.ui.stage.pan_tool = tool == ToolId::Hand;
                self.status(tool.label());
            }
            None => self.status(format!("the {id} tool is not available yet")),
        }
    }

    // ---- properties -------------------------------------------------------------------------------------------

    fn property_edit(&mut self, edit: &PropertyEdit) {
        let key = format!("prop:{}:{}", edit.id, self.prop_gesture);
        let id = edit.id.clone();
        let value = edit.value.clone();
        let result = self.host.session_mut().edit(&format!("Change {}", edit.id), Some(&key), |doc| crate::props::apply(doc, &id, &value));
        if edit.commit {
            self.prop_gesture = self.prop_gesture.saturating_add(1);
        }
        if let Err(e) = result {
            // The step recorded nothing useful; take it back.
            self.host.session_mut().undo();
            self.status(e);
        }
    }

    // ---- transport and playback -------------------------------------------------------------------------------

    fn transport_event(&mut self, event: TransportEvent) {
        match event {
            TransportEvent::Play => {
                if self.ui.playback.is_none() {
                    self.toggle_play();
                }
            }
            TransportEvent::Pause => self.stop_playback(),
            TransportEvent::GoToStart => self.goto_frame(0),
            TransportEvent::GoToEnd => {
                let last = self.frame_count().saturating_sub(1);
                self.goto_frame(last);
            }
            TransportEvent::StepBack => self.step(-1),
            TransportEvent::StepForward => self.step(1),
            TransportEvent::SetLoop(on) => self.ui.looping = on,
            TransportEvent::SetFps(fps) => {
                let fps = f64::from(fps).clamp(1.0, 240.0);
                self.host.session_mut().edit("Change Frame Rate", Some("fps"), |doc| doc.editor.document.frame_rate = fps);
            }
            TransportEvent::SetFrame(frame) => self.goto_frame(frame),
        }
    }

    pub(crate) fn frame_count(&self) -> u32 {
        let doc = self.host.session().doc();
        doc.document().timeline(&doc.editor.state.timeline).map_or(1, |t| t.frame_count().max(1))
    }

    fn step(&mut self, delta: i64) {
        let frame = i64::from(self.host.session().doc().editor.state.frame).saturating_add(delta);
        let total = i64::from(self.frame_count());
        let wrapped = if total > 0 { frame.rem_euclid(total) } else { 0 };
        self.goto_frame(u32::try_from(wrapped).unwrap_or(0));
    }

    pub(crate) fn toggle_play(&mut self) {
        if self.ui.playback.is_some() {
            self.stop_playback();
            return;
        }
        let from_frame = {
            let frame = self.host.session().doc().editor.state.frame;
            if frame.saturating_add(1) >= self.frame_count() { 0 } else { frame }
        };
        self.start_playback(from_frame);
    }

    fn start_playback(&mut self, from_frame: u32) {
        let token = self.ui.next_token;
        self.ui.next_token = token.saturating_add(1);
        self.ui.playback = Some(Playback { started: Instant::now(), from_frame, token });
        self.soundtrack_id = token;
        self.goto_frame(from_frame);
        let scene = match &self.host.session().doc().editor.state.timeline {
            TimelineId::Scene(i) => *i,
            TimelineId::Symbol(_) => self.host.session().doc().document().current_timeline,
        };
        self.worker.send(Job::Soundtrack { id: token, scene, from_frame });
    }

    pub(crate) fn stop_playback(&mut self) {
        if self.ui.playback.take().is_some() {
            self.audio.pause();
        }
    }

    pub(crate) fn soundtrack_ready(&mut self, id: u64, track: Option<anim_audio::PcmBuffer>, notes: &[String]) {
        if let Some(note) = notes.first() {
            self.status(format!("sound: {note}"));
        }
        let Some(pb) = self.ui.playback else { return };
        if pb.token != id {
            return;
        }
        if let Some(track) = track {
            self.audio.play(track, pb.started.elapsed().as_secs_f64());
        }
    }

    pub(crate) fn tick_playback(&mut self, ctx: &Context) {
        self.audio.poll();
        let Some(pb) = self.ui.playback else { return };
        let fps = self.host.session().doc().document().frame_rate.max(1.0);
        let advanced = u32::try_from(num::floor_f64_to_i32(pb.started.elapsed().as_secs_f64() * fps)).unwrap_or(0);
        let frame = pb.from_frame.saturating_add(advanced);
        let total = self.frame_count();
        if frame >= total {
            if self.ui.looping {
                self.start_playback(0);
            } else {
                self.stop_playback();
                self.goto_frame(total.saturating_sub(1));
            }
        } else {
            self.goto_frame(frame);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(5));
    }

    // ---- clipboard --------------------------------------------------------------------------------------------

    fn copy_selection(&mut self) {
        let doc = self.host.session().doc();
        let id = doc.editor.state.timeline.clone();
        let elements: Vec<Element> =
            doc.editor.state.selection.items.iter().filter_map(|r| anicraft_tools::resolve_ref(doc.document(), &id, r)).map(|r| r.element.clone()).collect();
        let count = elements.len();
        if count > 0 {
            self.ui.clipboard = elements;
            self.status(format!("copied {count} object(s)"));
        }
    }

    fn paste(&mut self) {
        if self.ui.clipboard.is_empty() {
            self.status("the clipboard is empty");
            return;
        }
        let elements = self.ui.clipboard.clone();
        let (timeline, layer, frame) = {
            let s = &self.host.session().doc().editor.state;
            (s.timeline.clone(), s.layer, s.frame)
        };
        let result = self
            .host
            .session_mut()
            .edit("Paste", None, |doc| crate::commands::paste_elements(&mut doc.editor.document, &timeline, layer, frame, elements, Point::new(12.0, 12.0)));
        match result {
            Ok(indices) => {
                let keyframe = self.keyframe_index(layer, frame);
                self.host.session_mut().doc_mut().editor.state.selection =
                    Selection { items: indices.into_iter().map(|i| ElementRef::top_level(layer, keyframe, i)).collect(), scope: None };
            }
            Err(e) => {
                self.host.session_mut().undo();
                self.status(format!("paste failed: {e}"));
            }
        }
    }

    // ---- timeline ---------------------------------------------------------------------------------------------

    fn timeline_event(&mut self, event: TimelineEvent) {
        self.ui.timeline.apply(&event);
        match event {
            TimelineEvent::SetFrame(frame) => self.goto_frame(frame),
            TimelineEvent::SelectFrames(selection) => {
                if let Some(layer) = selection.layers.first().copied() {
                    self.set_current_layer(layer);
                }
            }
            TimelineEvent::MoveFrames { selection, delta_frames, delta_rows: _ } => self.move_frames(&selection, delta_frames),
            TimelineEvent::FrameAction(action) => self.frame_action(action),
            TimelineEvent::Layer(event) => self.layer_event(event),
            TimelineEvent::SetOnionRange { before, after } => {
                self.ui.onion.before = before;
                self.ui.onion.after = after;
            }
            TimelineEvent::ToggleOnion => self.ui.onion.enabled = !self.ui.onion.enabled,
            TimelineEvent::ToggleOnionOutlines => self.ui.onion.outlines = !self.ui.onion.outlines,
            // Scrolling, zooming, the Frame View menu and the two mode toggles are view state, applied above;
            // `sync_modes` hands the modes to the editor.
            TimelineEvent::Scroll { .. }
            | TimelineEvent::Zoom { .. }
            | TimelineEvent::SetFrameView { .. }
            | TimelineEvent::ToggleEditMultiple
            | TimelineEvent::ToggleAutoKeyframe => {}
        }
    }

    /// Hands the timeline's mode toggles to the editor: auto keyframe, and Edit Multiple Frames over the range the
    /// onion skin markers span around the playhead.
    pub(super) fn sync_modes(&mut self) {
        let (auto, multiple) = (self.ui.timeline.auto_keyframe, self.ui.timeline.edit_multiple);
        let (before, after) = (self.ui.onion.before, self.ui.onion.after);
        let state = &mut self.host.session_mut().doc_mut().editor.state;
        state.auto_keyframe = auto;
        state.edit_range = multiple.then(|| (state.frame.saturating_sub(before), state.frame.saturating_add(after)));
    }

    fn set_current_layer(&mut self, id: u64) {
        if let Ok(layer) = usize::try_from(id) {
            self.host.session_mut().doc_mut().editor.state.layer = layer;
        }
    }

    /// The layers and frame range an operation applies to: the frame selection, or the current layer and playhead.
    fn timeline_target(&self) -> (Vec<usize>, u32, u32) {
        let doc = self.host.session().doc();
        let sel = &self.ui.timeline.selection;
        let layers: Vec<usize> = sel.layers.iter().filter_map(|l| usize::try_from(*l).ok()).collect();
        if layers.is_empty() {
            (vec![doc.editor.state.layer], doc.editor.state.frame, doc.editor.state.frame)
        } else {
            (layers, sel.first.min(sel.last), sel.first.max(sel.last))
        }
    }

    pub(crate) fn frame_op(&mut self, op: FrameOp) {
        let (layers, first, last) = self.timeline_target();
        let id = self.host.session().doc().editor.state.timeline.clone();
        let count = last.saturating_sub(first).saturating_add(1);
        let result = self.host.session_mut().edit(op.label(), None, |doc| -> Result<(), String> {
            let document = &mut doc.editor.document;
            for &layer in &layers {
                match op {
                    FrameOp::InsertFrames => edit::insert_frames(document, &id, layer, first, count),
                    FrameOp::RemoveFrames => edit::remove_frames(document, &id, layer, first, count),
                    FrameOp::InsertKeyframe => (first..=last).try_for_each(|f| edit::insert_keyframe(document, &id, layer, f)),
                    FrameOp::InsertBlankKeyframe => (first..=last).try_for_each(|f| edit::insert_blank_keyframe(document, &id, layer, f)),
                    FrameOp::ClearKeyframe => (first..=last).try_for_each(|f| edit::clear_keyframe(document, &id, layer, f)),
                    FrameOp::ClassicTween => edit::create_classic_tween(document, &id, layer, first),
                    FrameOp::ShapeTween => edit::create_shape_tween(document, &id, layer, first),
                    FrameOp::RemoveTween => edit::remove_tween(document, &id, layer, first),
                }
                .map_err(|e| format!("{}: {e}", op.label()))?;
            }
            Ok(())
        });
        if let Err(e) = result {
            self.host.session_mut().undo();
            self.status(e);
        }
    }

    fn frame_action(&mut self, action: FrameAction) {
        match action {
            FrameAction::InsertFrame => self.frame_op(FrameOp::InsertFrames),
            FrameAction::RemoveFrames => self.frame_op(FrameOp::RemoveFrames),
            FrameAction::InsertKeyframe => self.frame_op(FrameOp::InsertKeyframe),
            FrameAction::InsertBlankKeyframe => self.frame_op(FrameOp::InsertBlankKeyframe),
            FrameAction::ClearKeyframe => self.frame_op(FrameOp::ClearKeyframe),
            FrameAction::ConvertToKeyframes => {
                self.frame_range_command("timeline.convert_to_keyframes", json!({}));
            }
            FrameAction::ConvertToBlankKeyframes => {
                self.frame_range_command("timeline.convert_to_keyframes", json!({ "blank": true }));
            }
            FrameAction::ClearFrames => {
                self.frame_range_command("timeline.clear_frames", json!({}));
            }
            FrameAction::CreateClassicTween => self.frame_op(FrameOp::ClassicTween),
            FrameAction::CreateShapeTween => self.frame_op(FrameOp::ShapeTween),
            FrameAction::RemoveTween => self.frame_op(FrameOp::RemoveTween),
            FrameAction::CreateMotionTween => self.status("motion tweens are not supported yet; use a classic tween"),
            FrameAction::ReverseFrames => {
                self.frame_range_command("timeline.reverse_frames", json!({}));
            }
            FrameAction::CopyFrames => self.copy_frames(false),
            FrameAction::CutFrames => self.copy_frames(true),
            FrameAction::PasteFrames => self.paste_frames(false),
            FrameAction::PasteOverwrite => self.paste_frames(true),
            FrameAction::Properties | FrameAction::EditLabel => self.open_frame_properties(),
            FrameAction::SelectAllFrames => {
                let doc = self.host.session().doc();
                let layers = doc.document().timeline(&doc.editor.state.timeline).map_or(0, |t| t.layers.len());
                let frames = self.frame_count();
                self.ui.timeline.selection =
                    FrameSelection { layers: (0..layers).filter_map(|l| u64::try_from(l).ok()).collect(), first: 0, last: frames.saturating_sub(1) };
            }
        }
    }

    /// Runs a frame-range command (1-based frames, the layers of the selection) with `extra` params merged in.
    pub(crate) fn frame_range_command(&mut self, id: &str, extra: Value) -> bool {
        let (layers, first, last) = self.timeline_target();
        let mut params = json!({ "layers": layers, "from": first.saturating_add(1), "to": last.saturating_add(1) });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        self.run(id, &params)
    }

    /// Copy Frames / Cut Frames: the selected frames become the frame clipboard.
    fn copy_frames(&mut self, cut: bool) {
        let (layers, first, last) = self.timeline_target();
        let id = if cut { "timeline.cut_frames" } else { "timeline.copy_frames" };
        let params = json!({ "layers": layers, "from": first.saturating_add(1), "to": last.saturating_add(1) });
        let result = self.host.session_mut().execute(id, &params);
        match result {
            Ok(value) => {
                let clip = value.get("clip").cloned();
                let count = clip
                    .as_ref()
                    .and_then(|c| c.get("rows"))
                    .and_then(Value::as_array)
                    .map_or(0, |rows| rows.iter().filter_map(Value::as_array).map(Vec::len).sum::<usize>());
                self.ui.frame_clip = clip;
                self.status(format!("{} {count} keyframe(s)", if cut { "cut" } else { "copied" }));
            }
            Err(e) => self.status(format!("{id}: {e}")),
        }
    }

    /// Paste Frames (pushing what is there to the right) or Paste and Overwrite Frames, at the first selected frame.
    fn paste_frames(&mut self, overwrite: bool) {
        let Some(clip) = self.ui.frame_clip.clone() else {
            self.status("no frames were copied");
            return;
        };
        let (layers, first, _) = self.timeline_target();
        let layer = layers.iter().copied().min().unwrap_or(0);
        self.run("timeline.paste_frames", &json!({ "layer": layer, "frame": first.saturating_add(1), "overwrite": overwrite, "clip": clip }));
    }

    /// Opens the Frame Properties dialog for the keyframe under the playhead on the current layer.
    fn open_frame_properties(&mut self) {
        let (layers, first, _) = self.timeline_target();
        let layer = layers.first().copied().unwrap_or(0);
        let doc = self.host.session().doc();
        let Some(frame) = doc.document().timeline(&doc.editor.state.timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(first)) else {
            self.status("select a frame on a layer first");
            return;
        };
        let sound = frame.sound.as_ref();
        let props = FrameProps {
            layer,
            frame: frame.index,
            label: frame.label.clone().unwrap_or_default(),
            label_kind: match frame.label_type {
                None | Some(anim_scene::timeline::LabelType::Name) => 0,
                Some(anim_scene::timeline::LabelType::Comment) => 1,
                Some(anim_scene::timeline::LabelType::Anchor) => 2,
            },
            tween: format!("{:?}", frame.tween_type),
            ease: frame.acceleration,
            sound: sound.map(|s| s.name.clone()).unwrap_or_default(),
            sync: sound.map_or(0, |s| match s.sync {
                anim_scene::timeline::SoundSync::Event => 0,
                anim_scene::timeline::SoundSync::Start => 1,
                anim_scene::timeline::SoundSync::Stop => 2,
                anim_scene::timeline::SoundSync::Stream => 3,
            }),
            repeat: sound.map_or(1, |s| match s.loop_mode {
                anim_scene::timeline::SoundLoop::Repeat(n) => n,
                _ => 1,
            }),
            forever: sound.is_some_and(|s| matches!(s.loop_mode, anim_scene::timeline::SoundLoop::Forever)),
            script: frame.actionscript.clone().unwrap_or_default(),
            sounds: doc.document().library.items.iter().filter_map(|i| if let LibraryItem::Sound(s) = i { Some(s.name.clone()) } else { None }).collect(),
        };
        self.ui.dialog = Some(Dialog::FrameProperties(props));
    }

    /// Applies the Frame Properties dialog: label, ease, sound and script, as one undo step each command.
    fn apply_frame_properties(&mut self, p: &FrameProps) {
        let frame = p.frame.saturating_add(1);
        let layer = p.layer;
        let kinds = ["name", "comment", "anchor"];
        let syncs = ["event", "start", "stop", "stream"];
        self.run("timeline.set_label", &json!({ "layer": layer, "frame": frame, "label": p.label, "kind": kinds.get(p.label_kind).copied().unwrap_or("name") }));
        if p.tween != "None" {
            self.run("timeline.set_ease", &json!({ "layer": layer, "frame": frame, "acceleration": p.ease }));
        }
        let sound = if p.sound.is_empty() {
            json!({ "layer": layer, "frame": frame })
        } else {
            json!({ "layer": layer, "frame": frame, "sound": p.sound, "sync": syncs.get(p.sync).copied().unwrap_or("event"), "repeat": p.repeat.max(1), "forever": p.forever })
        };
        self.run("timeline.set_sound", &sound);
        self.run("timeline.set_script", &json!({ "layer": layer, "frame": frame, "script": p.script }));
    }

    fn move_frames(&mut self, selection: &FrameSelection, delta: i32) {
        if delta == 0 {
            return;
        }
        let id = self.host.session().doc().editor.state.timeline.clone();
        let layers: Vec<usize> = selection.layers.iter().filter_map(|l| usize::try_from(*l).ok()).collect();
        let (lo, hi) = (selection.first.min(selection.last), selection.first.max(selection.last));
        let result = self.host.session_mut().edit("Move Frames", None, |doc| -> Result<(), String> {
            let document = &mut doc.editor.document;
            for &layer in &layers {
                let mut keys: Vec<u32> = document
                    .timeline(&id)
                    .and_then(|t| t.layers.get(layer))
                    .map(|l| l.frames.iter().map(|f| f.index).filter(|i| *i >= lo && *i <= hi).collect())
                    .unwrap_or_default();
                // Move the keyframes furthest in the direction of travel first so they never run into each other.
                if delta > 0 {
                    keys.reverse();
                }
                for from in keys {
                    let to = i64::from(from).saturating_add(i64::from(delta));
                    let to = u32::try_from(to).map_err(|_| "cannot move before the first frame".to_owned())?;
                    edit::move_keyframe(document, &id, layer, from, to).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                let shift = |v: u32| u32::try_from(i64::from(v).saturating_add(i64::from(delta))).unwrap_or(0);
                self.ui.timeline.selection.first = shift(selection.first);
                self.ui.timeline.selection.last = shift(selection.last);
            }
            Err(e) => {
                self.host.session_mut().undo();
                self.status(e);
            }
        }
    }

    fn add_layer(&mut self, kind: LayerKind) {
        let id = self.host.session().doc().editor.state.timeline.clone();
        let at = self.host.session().doc().editor.state.layer;
        let result = self.host.session_mut().edit("Add Layer", None, |doc| edit::add_layer(&mut doc.editor.document, &id, at, None, kind));
        match result {
            Ok(index) => {
                self.host.session_mut().doc_mut().editor.state.layer = index;
                self.ui.timeline.selected_layers = vec![u64::try_from(index).unwrap_or(0)];
            }
            Err(e) => {
                self.host.session_mut().undo();
                self.status(e.to_string());
            }
        }
    }

    fn layer_event(&mut self, event: LayerEvent) {
        let id = self.host.session().doc().editor.state.timeline.clone();
        let idx = |l: u64| usize::try_from(l).unwrap_or(usize::MAX);
        match event {
            LayerEvent::Select { id: layer, .. } => self.set_current_layer(layer),
            LayerEvent::ToggleVisible(l) => self.layer_flag(&id, idx(l), "Toggle Layer Visibility", |layer| layer.visible = !layer.visible),
            LayerEvent::ToggleLocked(l) => self.layer_flag(&id, idx(l), "Toggle Layer Lock", |layer| layer.locked = !layer.locked),
            LayerEvent::ToggleOutline(l) => self.layer_flag(&id, idx(l), "Toggle Layer Outline", |layer| layer.outline = !layer.outline),
            LayerEvent::ToggleFolder(l) => {
                if !self.ui.collapsed_layers.remove(&l) {
                    self.ui.collapsed_layers.insert(l);
                }
            }
            LayerEvent::RenameRequested(l) => {
                let name =
                    self.host.session().doc().document().timeline(&id).and_then(|t| t.layers.get(idx(l))).map(|layer| layer.name.clone()).unwrap_or_default();
                self.ui.dialog = Some(Dialog::RenameLayer { index: idx(l), name });
            }
            LayerEvent::SetAllVisible(on) => self.all_layers(&id, "Show/Hide All Layers", |layer| layer.visible = on),
            LayerEvent::SetAllLocked(on) => self.all_layers(&id, "Lock/Unlock All Layers", |layer| layer.locked = on),
            LayerEvent::SetAllOutline(on) => self.all_layers(&id, "Outline All Layers", |layer| layer.outline = on),
            LayerEvent::Add => self.add_layer(LayerKind::Normal),
            LayerEvent::AddFolder => self.add_layer(LayerKind::Folder),
            LayerEvent::AddKind(kind) => self.add_layer(scene_kind(kind)),
            LayerEvent::AddMotionGuide(l) => {
                if self.run("layer.add_motion_guide", &json!({ "layer": idx(l) })) {
                    self.status("added a motion guide layer");
                }
            }
            LayerEvent::Convert { id: l, kind } => {
                self.run("layer.set_kind", &json!({ "layer": idx(l), "kind": kind_name(scene_kind(kind)) }));
            }
            LayerEvent::Delete(l) => {
                let result = self.host.session_mut().edit("Delete Layer", None, |doc| edit::remove_layer(&mut doc.editor.document, &id, idx(l)));
                match result {
                    Ok(remaining) => {
                        let current = self.host.session().doc().editor.state.layer;
                        self.host.session_mut().doc_mut().editor.state.layer = current.min(remaining.saturating_sub(1));
                        self.ui.timeline.selection = FrameSelection::default();
                    }
                    Err(e) => {
                        self.host.session_mut().undo();
                        self.status(e.to_string());
                    }
                }
            }
            LayerEvent::Duplicate(l) => {
                let result = self.host.session_mut().edit("Duplicate Layer", None, |doc| edit::duplicate_layer(&mut doc.editor.document, &id, idx(l)));
                if let Err(e) = result {
                    self.host.session_mut().undo();
                    self.status(e.to_string());
                }
            }
            LayerEvent::Properties(l) => self.open_layer_properties(idx(l)),
            LayerEvent::Move { id: moving, target, placement } => {
                let (from, target) = (idx(moving), idx(target));
                let to = match placement {
                    DropPlacement::Before => target,
                    DropPlacement::After | DropPlacement::Inside => target.saturating_add(1),
                };
                let to = if from < to { to.saturating_sub(1) } else { to };
                let result = self.host.session_mut().edit("Move Layer", None, |doc| edit::move_layer(&mut doc.editor.document, &id, from, to));
                match result {
                    Ok(_) => {
                        if placement == DropPlacement::Inside {
                            let parent = if from < target { target.saturating_sub(1) } else { target };
                            let _ = self
                                .host
                                .session_mut()
                                .edit("Move Layer into Folder", None, |doc| edit::set_layer_parent(&mut doc.editor.document, &id, to, Some(parent)));
                        }
                    }
                    Err(e) => {
                        self.host.session_mut().undo();
                        self.status(e.to_string());
                    }
                }
            }
        }
    }

    /// Opens the Layer Properties dialog for the layer at `index`.
    pub(crate) fn open_layer_properties(&mut self, index: usize) {
        let doc = self.host.session().doc();
        let Some(layer) = doc.document().timeline(&doc.editor.state.timeline).and_then(|t| t.layers.get(index)) else { return };
        self.ui.dialog = Some(Dialog::LayerProperties(LayerProps {
            index,
            name: anicraft_xfl::display_name(&layer.name).into_owned(),
            kind: match layer.kind {
                LayerKind::Guide => 1,
                LayerKind::Mask => 2,
                LayerKind::Folder => 3,
                _ => 0,
            },
            color: [layer.color.r, layer.color.g, layer.color.b],
            height: anicraft_cli::mcp::timeline_ops::layer_height(layer),
            visible: layer.visible,
            locked: layer.locked,
            outline: layer.outline,
        }));
    }

    // ---- scenes -----------------------------------------------------------------------------------------------

    /// The Scene panel and the edit bar's scene menu: shows scene `index` (0-based) for editing.
    pub(crate) fn go_to_scene(&mut self, index: usize) {
        if index >= self.host.session().doc().document().scenes.len() {
            return;
        }
        self.stop_playback();
        if self.run("editor.goto", &json!({ "scene": index, "frame": 0 })) {
            self.ui.timeline.selection = FrameSelection::default();
        }
    }

    // ---- window modes -----------------------------------------------------------------------------------------

    fn layer_flag(&mut self, id: &TimelineId, layer: usize, label: &str, f: impl FnOnce(&mut anim_scene::Layer)) {
        self.host.session_mut().edit(label, None, |doc| {
            if let Some(l) = doc.editor.document.timeline_mut(id).and_then(|t| t.layers.get_mut(layer)) {
                f(l);
            }
        });
    }

    fn all_layers(&mut self, id: &TimelineId, label: &str, f: impl Fn(&mut anim_scene::Layer)) {
        self.host.session_mut().edit(label, None, |doc| {
            if let Some(t) = doc.editor.document.timeline_mut(id) {
                t.layers.iter_mut().for_each(&f);
            }
        });
    }

    // ---- library ----------------------------------------------------------------------------------------------

    fn library_event(&mut self, event: LibraryEvent) {
        match event {
            LibraryEvent::Select(id) => self.ui.library_selected = self.library_name(id),
            LibraryEvent::PreviewPlay => self.preview_play(),
            LibraryEvent::PreviewStop => self.preview_stop(),
            LibraryEvent::PreviewSeek(permille) => self.preview_seek(permille),
            LibraryEvent::Open(id) | LibraryEvent::Action { id, action: LibraryAction::Edit } => {
                if let Some(name) = self.library_name(id) {
                    self.edit_symbol(&name);
                }
            }
            LibraryEvent::ToggleFolder(id) => {
                if let Some(name) = self.library_name(id)
                    && !self.ui.collapsed_folders.remove(&name)
                {
                    self.ui.collapsed_folders.insert(name);
                }
            }
            LibraryEvent::FilterChanged(text) => self.ui.library_filter = text,
            LibraryEvent::Action { id, action } => {
                let Some(name) = self.library_name(id) else { return };
                match action {
                    LibraryAction::Rename => {
                        let leaf = name.rsplit('/').next().unwrap_or(&name).to_owned();
                        self.ui.dialog = Some(Dialog::RenameItem { old: name, name: leaf });
                    }
                    LibraryAction::Duplicate => {
                        let new_name = format!("{name} copy");
                        self.run("library.duplicate_symbol", &json!({ "name": name, "new_name": new_name }));
                    }
                    LibraryAction::Delete => {
                        self.run("library.remove_item", &json!({ "name": name }));
                    }
                    LibraryAction::Properties => self.status(format!("{name}: {}", self.describe_item(&name))),
                    LibraryAction::Edit => {}
                }
            }
            LibraryEvent::NewSymbol => self.ui.dialog = Some(Dialog::NewSymbol { name: "Symbol 1".to_owned(), kind: 0 }),
            LibraryEvent::NewFolder => {
                let result = self.host.session_mut().edit("New Folder", None, |doc| {
                    let lib = &mut doc.editor.document.library;
                    let mut n = 1;
                    let name = loop {
                        let candidate = format!("New Folder {n}");
                        if lib.find_index(&candidate).is_none() {
                            break candidate;
                        }
                        n = usize::saturating_add(n, 1);
                    };
                    let item_id = lib.fresh_item_id(&name);
                    lib.push(LibraryItem::Folder(anim_scene::FolderItem { name, item_id, is_expanded: true, extra: anim_scene::Extra::new() }))
                });
                if let Err(e) = result {
                    self.host.session_mut().undo();
                    self.status(e.to_string());
                }
            }
        }
    }

    fn describe_item(&self, name: &str) -> String {
        let doc = self.host.session().doc().document();
        match doc.library.find(name) {
            Some(LibraryItem::Symbol(s)) => format!("{:?} symbol, {} frames, {} layers", s.kind, s.timeline.frame_count(), s.timeline.layers.len()),
            Some(LibraryItem::Bitmap(b)) => format!("bitmap {:?}", b.size),
            Some(other) => other.kind_name().to_owned(),
            None => "missing".to_owned(),
        }
    }

    /// Edit Symbols: a symbol's timeline on a stage of its own; the edit bar returns.
    pub(crate) fn edit_symbol(&mut self, name: &str) {
        if self.run("edit.enter_symbol", &json!({ "name": name })) {
            self.ui.timeline.selection = FrameSelection::default();
            self.stop_playback();
        }
    }

    /// Edit in Place: the selected symbol instance edited where it sits, the rest of the stage dimmed.
    pub(crate) fn edit_selected_in_place(&mut self) {
        let doc = self.host.session().doc();
        let Some(target) = doc.editor.state.selection.items.first().cloned() else {
            self.status("select a symbol instance first");
            return;
        };
        let Some(resolved) = anicraft_tools::resolve_ref(doc.document(), &doc.editor.state.timeline, &target) else { return };
        let Element::SymbolInstance(instance) = resolved.element else {
            self.status("select a symbol instance first");
            return;
        };
        let name = instance.library_item_name.clone();
        let m = resolved.local_to_stage().as_coeffs();
        if self.run("edit.enter_symbol", &json!({ "name": name, "in_place": true, "matrix": m })) {
            self.ui.timeline.selection = FrameSelection::default();
            self.stop_playback();
        }
    }

    // ---- dialogs ----------------------------------------------------------------------------------------------

    /// Applies a confirmed dialog.
    pub(super) fn dialog_ok(&mut self, dialog: Dialog) {
        let kinds = ["graphic", "movie_clip", "button"];
        match dialog {
            Dialog::ConvertToSymbol { name, kind } => {
                self.run("modify.convert_to_symbol", &json!({ "name": name, "kind": kinds.get(kind).copied().unwrap_or("graphic") }));
            }
            Dialog::NewSymbol { name, kind } => {
                if self.run("insert.symbol", &json!({ "name": name, "kind": kinds.get(kind).copied().unwrap_or("graphic") })) {
                    self.edit_symbol(&name);
                }
            }
            Dialog::RenameLayer { index, name } => {
                let id = self.host.session().doc().editor.state.timeline.clone();
                let result = self.host.session_mut().edit("Rename Layer", None, |doc| edit::rename_layer(&mut doc.editor.document, &id, index, &name));
                if let Err(e) = result {
                    self.host.session_mut().undo();
                    self.status(e.to_string());
                }
            }
            Dialog::RenameScene { index, name } => {
                self.run("scene.rename", &json!({ "scene": index, "name": name }));
            }
            Dialog::LayerProperties(p) => {
                let kinds = ["normal", "guide", "mask", "folder"];
                self.run(
                    "layer.set_properties",
                    &json!({
                        "layer": p.index,
                        "name": p.name,
                        "kind": kinds.get(p.kind).copied().unwrap_or("normal"),
                        "color": format!("#{:02x}{:02x}{:02x}", p.color[0], p.color[1], p.color[2]),
                        "height": p.height,
                        "visible": p.visible,
                        "locked": p.locked,
                        "outline": p.outline,
                    }),
                );
            }
            Dialog::FrameProperties(p) => self.apply_frame_properties(&p),
            Dialog::RenameItem { old, name } => {
                let new_name = old.rsplit_once('/').map_or_else(|| name.clone(), |(parent, _)| format!("{parent}/{name}"));
                self.run("library.rename_item", &json!({ "name": old, "new_name": new_name }));
            }
            Dialog::DocumentSettings | Dialog::About | Dialog::Message { .. } => {}
            Dialog::Discard(action) => self.discard_confirmed(*action),
        }
    }

    pub(crate) fn recent_paths(&self) -> Vec<PathBuf> {
        self.ui.recent.clone()
    }

    pub(crate) fn finish_export_status(&mut self, label: &str, fraction: f32) {
        self.ui.export = Some(ExportStatus { label: label.to_owned(), fraction });
    }
}
