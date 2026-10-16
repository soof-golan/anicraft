//! Pointer and keyboard input: stage pointer events go to the active tool, keys go to shortcuts or the tool.

use anicraft_tools::PointerKind as K;
use anicraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, ToolCommand, ToolContext, ToolEvent, ToolId};
use anim_core::Point;
use anim_session::Shortcut;
use anim_ui::stage::{PointerPhase, StagePointer};
use anim_ui::timeline::FrameAction;
use egui::{Context, PointerButton, Vec2};
use serde_json::json;

use super::AniApp;
use super::actions::FrameOp;
use crate::docs::DocAction;
use crate::menus::MenuAction;
use crate::state::Dialog;
use crate::view;

fn tool_modifiers(m: egui::Modifiers) -> Modifiers {
    Modifiers { shift: m.shift, ctrl: m.command, alt: m.alt }
}

fn tool_key(key: egui::Key) -> Option<Key> {
    Some(match key {
        egui::Key::Escape => Key::Escape,
        egui::Key::Enter => Key::Enter,
        egui::Key::Backspace => Key::Backspace,
        egui::Key::Delete => Key::Delete,
        egui::Key::Tab => Key::Tab,
        egui::Key::Space => Key::Space,
        egui::Key::ArrowLeft => Key::Left,
        egui::Key::ArrowRight => Key::Right,
        egui::Key::ArrowUp => Key::Up,
        egui::Key::ArrowDown => Key::Down,
        egui::Key::Home => Key::Home,
        egui::Key::End => Key::End,
        other => {
            let name = other.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
    })
}

impl AniApp {
    /// Motion path points and shape hints on the stage are grabbed before any tool sees the pointer. Returns true
    /// when the event was theirs.
    fn motion_pointer(&mut self, p: &StagePointer) -> bool {
        use crate::motion_ui::{Drag, drop_command, handles, pick, tween_view};
        match p.phase {
            PointerPhase::Down => {
                let items = handles(self.host.session().doc());
                match pick(&items, p.origin, p.zoom) {
                    Some(handle) => {
                        self.ui.motion.drag = Some(Drag { handle, pos: p.pos });
                        true
                    }
                    None => false,
                }
            }
            PointerPhase::Drag => self.ui.motion.drag.as_mut().is_some_and(|d| {
                d.pos = p.pos;
                true
            }),
            PointerPhase::Up => {
                let Some(drag) = self.ui.motion.drag.take() else { return false };
                if let Some(view) = tween_view(self.host.session().doc()) {
                    let (id, params) = drop_command(&view, drag.handle, p.pos);
                    self.run(id, &params);
                }
                true
            }
            PointerPhase::Click | PointerPhase::DoubleClick => pick(&handles(self.host.session().doc()), p.pos, p.zoom).is_some(),
            PointerPhase::Move => false,
        }
    }

    /// A pointer event of the stage widget, as the tools see it.
    pub(super) fn stage_pointer(&mut self, p: &StagePointer) {
        if self.ui.test_movie.is_some() {
            self.test_movie_pointer(p);
            return;
        }
        if p.button == PointerButton::Secondary && p.phase == PointerPhase::Click {
            self.open_stage_menu(Point::new(f64::from(p.pos.x), f64::from(p.pos.y)));
            return;
        }
        if p.button != PointerButton::Primary && p.phase != PointerPhase::Move {
            return;
        }
        if self.motion_pointer(p) {
            return;
        }
        let modifiers = tool_modifiers(p.modifiers);
        let mut pos = Point::new(f64::from(p.pos.x), f64::from(p.pos.y));
        // A symbol edited in place sits at a matrix inside the stage; the tools work in the symbol's own space.
        if let Some(m) = self.host.session().doc().editor.state.in_place
            && m.determinant().abs() > 1e-12
        {
            pos = m.inverse() * pos;
        }
        let canvas = self.stage_canvas();
        let screen = self.ui.stage.view.to_screen(egui::Rect::from_min_size(egui::Pos2::ZERO, canvas.size()), p.pos);
        let event = |kind: anicraft_tools::PointerKind, count: u8| {
            ToolEvent::Pointer(
                PointerEvent::new(kind, pos.x, pos.y).with_modifiers(modifiers).with_click_count(count).with_screen(f64::from(screen.x), f64::from(screen.y)),
            )
        };
        match p.phase {
            PointerPhase::Move => self.tool_event(&event(K::Hover, 1)),
            PointerPhase::Down => self.tool_event(&event(K::Down, 1)),
            PointerPhase::Drag => self.tool_event(&event(K::Move, 1)),
            PointerPhase::Up => self.tool_event(&event(K::Up, 1)),
            PointerPhase::Click => {
                self.tool_event(&event(K::Down, 1));
                self.tool_event(&event(K::Up, 1));
            }
            PointerPhase::DoubleClick => {
                self.tool_event(&event(K::Down, 2));
                self.tool_event(&event(K::Up, 2));
            }
        }
    }

    /// Feeds one event to the active tool, applies what it asks for, and records one undo step per gesture.
    pub(crate) fn tool_event(&mut self, event: &ToolEvent) {
        let accent = self.ui.theme.palette().accent;
        let zoom = f64::from(self.ui.stage.view.zoom);
        let session = self.host.session_mut();
        let doc = session.doc_mut();
        doc.editor.state.view.zoom = zoom;
        let out = {
            let ed = &mut doc.editor;
            let bounds = anicraft_tools::LibraryBounds::new(&ed.document.library);
            let ctx = ToolContext {
                document: &ed.document,
                timeline: &ed.state.timeline,
                layer: ed.state.layer,
                frame: ed.state.frame,
                edit_range: ed.state.edit_range,
                selection: &ed.state.selection,
                settings: &ed.state.settings,
                zoom: ed.state.view.zoom,
                bounds: &bounds,
            };
            ed.tools.handle(event, &ctx)
        };
        let edits = out.commands.iter().any(ToolCommand::edits_document);
        if edits && self.ui.drag_base.is_none() {
            self.ui.drag_base = Some(Box::new(doc.clone()));
            doc.editor.tools.active().label().clone_into(&mut self.ui.drag_label);
        }
        let failure = doc.editor.run_commands(&out.commands).err().map(|e| e.to_string());
        doc.editor.state.active_tool = doc.editor.tools.active();
        let busy = doc.editor.tools.is_busy();
        let text_editing = doc.editor.state.text_editing.is_some();
        let tool = doc.editor.tools.active();
        let overlay = doc.editor.overlay();
        let mut finished_text = false;
        for command in &out.commands {
            match command {
                ToolCommand::PanScreen { delta } => {
                    self.ui.stage.view = self.ui.stage.view.panned(Vec2::new(anim_core::num::f64_to_f32(delta.x), anim_core::num::f64_to_f32(delta.y)));
                }
                ToolCommand::ZoomAt { center, factor } => {
                    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, self.stage_canvas().size());
                    let anchor = egui::pos2(anim_core::num::f64_to_f32(center.x), anim_core::num::f64_to_f32(center.y));
                    self.ui.stage.view =
                        self.ui.stage.view.zoom_about(canvas, self.ui.stage.view.to_screen(canvas, anchor), anim_core::num::f64_to_f32(*factor));
                }
                ToolCommand::ZoomToRect { rect } => {
                    let size = Vec2::new(anim_core::num::f64_to_f32(rect.width()), anim_core::num::f64_to_f32(rect.height()));
                    if size.x > 0.0 && size.y > 0.0 {
                        let canvas = self.stage_canvas().size();
                        let fit = anim_ui::StageView::fit(canvas, size, 24.0);
                        let center = egui::vec2(anim_core::num::f64_to_f32(rect.center().x), anim_core::num::f64_to_f32(rect.center().y));
                        self.ui.stage.view = anim_ui::StageView { zoom: fit.zoom, pan: canvas / 2.0 - center * fit.zoom };
                    }
                }
                ToolCommand::ActivateTool { tool } => self.ui.stage.pan_tool = *tool == ToolId::Hand,
                ToolCommand::EndTextEdit => finished_text = true,
                _ => {}
            }
        }
        let to_stage = self.host.session().doc().editor.state.in_place.unwrap_or(anim_core::Affine::IDENTITY);
        self.ui.tool_overlay = view::overlays(&overlay, accent, to_stage);
        self.ui.tool_cursor = Some(view::cursor_icon(out.cursor));
        let gesture_over = !busy
            && !text_editing
            && (matches!(event, ToolEvent::Pointer(p) if p.kind == anicraft_tools::PointerKind::Up) || matches!(event, ToolEvent::Key(_) | ToolEvent::Cancel));
        if (gesture_over || finished_text)
            && let Some(base) = self.ui.drag_base.take()
        {
            let label = std::mem::take(&mut self.ui.drag_label);
            self.host.session_mut().history_mut().record(*base, label, None);
            self.edits = self.edits.saturating_add(1);
        } else if edits {
            self.edits = self.edits.saturating_add(1);
        }
        let _ = tool;
        if let Some(message) = failure {
            self.status(message);
        }
    }

    /// Keyboard shortcuts and text input. Does nothing while a text box (not the stage) has the focus.
    pub(super) fn keyboard(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        let (typing, busy) = {
            let ed = &self.host.session().doc().editor;
            (ed.state.text_editing.is_some(), ed.tools.is_busy())
        };
        for event in events {
            match event {
                egui::Event::Text(text) if typing => self.tool_event(&ToolEvent::Text { text }),
                egui::Event::Paste(text) if typing => self.tool_event(&ToolEvent::Text { text }),
                egui::Event::Key { key, pressed, modifiers, repeat, .. } => {
                    if typing && !modifiers.command {
                        if let Some(k) = tool_key(key) {
                            self.tool_event(&ToolEvent::Key(KeyEvent { key: k, modifiers: tool_modifiers(modifiers), pressed }));
                        }
                    } else if pressed && !repeat {
                        self.shortcut(ctx, key, modifiers, busy);
                    } else if !pressed && key == egui::Key::Space {
                        self.tool_event(&ToolEvent::Key(KeyEvent::release(Key::Space)));
                    }
                }
                _ => {}
            }
        }
    }

    fn shortcut(&mut self, ctx: &Context, key: egui::Key, modifiers: egui::Modifiers, busy: bool) {
        use egui::Key as K;
        let chord = Shortcut { command: modifiers.command, shift: modifiers.shift, alt: modifiers.alt, key: key.name().to_owned() };
        // The Edit > Timeline chords come before the plain Ctrl ones they extend.
        let frames = match (modifiers.command, modifiers.alt, key) {
            (true, true, K::X) => Some(FrameAction::CutFrames),
            (true, true, K::C) => Some(FrameAction::CopyFrames),
            (true, true, K::V) if modifiers.shift => Some(FrameAction::PasteOverwrite),
            (true, true, K::V) => Some(FrameAction::PasteFrames),
            (true, true, K::A) => Some(FrameAction::SelectAllFrames),
            (false, true, K::Backspace) => Some(FrameAction::ClearFrames),
            _ => None,
        };
        if let Some(action) = frames {
            self.handle(ctx, super::actions::UiEvent::Menu(MenuAction::Frame(action)));
            return;
        }
        // Shape and arrange chords (Paste in Place, Lock, Distribute to Layers, Rotate 90, ...).
        let shape_chord = match (modifiers.command, modifiers.shift, modifiers.alt, key) {
            (true, true, false, K::V) => Some(MenuAction::PasteInPlace),
            (true, false, true, K::L) => Some(MenuAction::Command { id: "modify.lock", params: json!({}) }),
            (true, true, true, K::L) => Some(MenuAction::Command { id: "modify.unlock_all", params: json!({}) }),
            (true, true, false, K::D) => Some(MenuAction::Command { id: "modify.distribute_to_layers", params: json!({}) }),
            (true, true, false, K::Num9) => Some(MenuAction::Command { id: "modify.transform", params: json!({ "op": "rotate_90_cw" }) }),
            (true, true, false, K::Num7) => Some(MenuAction::Command { id: "modify.transform", params: json!({ "op": "rotate_90_ccw" }) }),
            (true, false, true, K::S) => Some(MenuAction::Dialog(Dialog::Shape(crate::shape_ui::ShapeDialog::scale_rotate()))),
            (true, true, true, K::C) => Some(MenuAction::Dialog(Dialog::Shape(crate::shape_ui::ShapeDialog::optimize()))),
            _ => None,
        };
        if let Some(action) = shape_chord {
            self.handle(ctx, super::actions::UiEvent::Menu(action));
            return;
        }
        // Application-level chords first.
        let action = match (modifiers.command, modifiers.shift, key) {
            (false, false, K::Escape) if self.ui.test_movie.is_some() => Some(MenuAction::Doc(DocAction::TestMovie)),
            (true, false, K::Enter) => Some(MenuAction::Doc(DocAction::TestMovie)),
            (false, false, K::F9) => Some(MenuAction::Doc(DocAction::TogglePanel(crate::docs::ACTIONS_PANEL))),
            (true, false, K::E) => Some(MenuAction::EditSymbol),
            (false, true, K::F2) => Some(MenuAction::ToggleScenePanel),
            (true, false, K::N) => Some(MenuAction::New),
            (true, false, K::O) => Some(MenuAction::Open),
            (true, false, K::S) => Some(MenuAction::Save),
            (true, true, K::S) => Some(MenuAction::SaveAs),
            (true, false, K::Q) => Some(MenuAction::Quit),
            (true, false, K::R) => Some(MenuAction::Import { to_stage: true }),
            (true, false, K::J) => Some(MenuAction::Doc(DocAction::OpenSettings)),
            (true, false, K::M) if modifiers.alt => Some(MenuAction::Doc(DocAction::Play(crate::docs::PlayToggle::MuteSounds))),
            (true, false, K::B) if modifiers.alt => Some(MenuAction::Doc(DocAction::Play(crate::docs::PlayToggle::SimpleButtons))),
            (true, false, K::F) => Some(MenuAction::Doc(DocAction::Find)),
            (true, true, K::W) => Some(MenuAction::Doc(DocAction::CloseAll)),
            (true, false, K::W) => Some(MenuAction::Doc(DocAction::CloseTab)),
            (true, false, K::PageDown) => Some(MenuAction::Doc(DocAction::NextTab)),
            (true, false, K::PageUp) => Some(MenuAction::Doc(DocAction::PrevTab)),
            (false, false, K::F3) => Some(MenuAction::Doc(DocAction::FindAgain)),
            (true, false, K::F10) => Some(MenuAction::Doc(DocAction::TogglePanel(crate::docs::HISTORY_PANEL))),
            (false, false, K::F2) => Some(MenuAction::Doc(DocAction::TogglePanel(anim_ui::PanelId::Output))),
            (true, false, K::C) => Some(MenuAction::Copy),
            (true, false, K::X) => Some(MenuAction::Cut),
            (true, false, K::V) => Some(MenuAction::Paste),
            (true, false, K::A) => Some(MenuAction::Command { id: "select.all", params: json!({}) }),
            (true, true, K::A) => Some(MenuAction::Command { id: "select.none", params: json!({}) }),
            (true, false, K::D) => Some(MenuAction::Command { id: "edit.duplicate", params: json!({}) }),
            (true, false, K::Z) => Some(MenuAction::Command { id: "edit.undo", params: json!({}) }),
            (true, true, K::Z) | (true, false, K::Y) => Some(MenuAction::Command { id: "edit.redo", params: json!({}) }),
            (true, false, K::K) => Some(MenuAction::Layout(crate::layout_ui::LayoutEvent::TogglePanel(crate::layout_ui::ALIGN_PANEL))),
            (true, false, K::T) => Some(MenuAction::Layout(crate::layout_ui::LayoutEvent::TogglePanel(crate::layout_ui::TRANSFORM_PANEL))),
            (true, false, K::Equals | K::Plus) => Some(MenuAction::ZoomIn),
            (true, false, K::Minus) => Some(MenuAction::ZoomOut),
            (true, false, K::Num0) => Some(MenuAction::ZoomFit),
            (true, false, K::Num1) => Some(MenuAction::Zoom100),
            (true, false, K::B) => Some(MenuAction::Command { id: "modify.break_apart", params: json!({}) }),
            (true, false, K::G) => Some(MenuAction::Command { id: "modify.group", params: json!({}) }),
            (true, true, K::G) => Some(MenuAction::Command { id: "modify.ungroup", params: json!({}) }),
            (true, true, K::H) => Some(MenuAction::Command { id: "shape.add_hint", params: json!({}) }),
            (false, false, K::Enter) if !busy => Some(MenuAction::Play),
            (false, false, K::Home) => Some(MenuAction::GoStart),
            (false, false, K::End) => Some(MenuAction::GoEnd),
            (false, false, K::Period) => Some(MenuAction::StepForward),
            (false, false, K::Comma) => Some(MenuAction::StepBack),
            (false, false, K::F1) => Some(MenuAction::Dialog(Dialog::About)),
            (false, false, K::F8) => Some(MenuAction::Dialog(Dialog::ConvertToSymbol { name: "Symbol 1".to_owned(), kind: 0 })),
            (false, false, K::Delete | K::Backspace) => Some(MenuAction::Command { id: "edit.delete", params: json!({}) }),
            _ => None,
        };
        if let Some(action) = action {
            self.handle(ctx, super::actions::UiEvent::Menu(action));
            return;
        }
        // Frame keys: F5 frame, F6 keyframe, F7 blank keyframe (Shift removes).
        let op = match (key, modifiers.shift) {
            (K::F5, false) => Some(FrameOp::InsertFrames),
            (K::F5, true) => Some(FrameOp::RemoveFrames),
            (K::F6, false) => Some(FrameOp::InsertKeyframe),
            (K::F6, true) => Some(FrameOp::ClearKeyframe),
            (K::F7, false) => Some(FrameOp::InsertBlankKeyframe),
            _ => None,
        };
        if let Some(op) = op {
            self.frame_op(op);
            return;
        }
        let registered = self.host.session().registry().lookup_shortcut(&chord).map(|spec| spec.id);
        if let Some(id) = registered {
            self.run(id, &json!({}));
            return;
        }
        // Plain keys go to the tools: single-letter tool shortcuts and the space bar.
        if let Some(k) = tool_key(key)
            && !modifiers.command
        {
            self.tool_event(&ToolEvent::Key(KeyEvent { key: k, modifiers: tool_modifiers(modifiers), pressed: true }));
            let active = self.host.session().doc().editor.state.active_tool;
            self.ui.stage.pan_tool = active == ToolId::Hand;
        }
    }
}
