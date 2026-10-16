//! The menu bar: File, Edit, View, Insert, Modify, Control, Help. A pure function of what it is told; it returns the
//! item that was picked and changes nothing.

use std::path::PathBuf;

use anicraft_cli::export::Format;
use anim_ui::PanelId;
use anim_ui::timeline::{FrameAction, LayerKind};
use egui::Ui;
use serde_json::{Value, json};

use crate::docs::{BUILTIN_CATEGORY_ORDER, DOC_PANELS, DocAction, PlayToggle, TemplateChoice, panel_title};
use crate::shape_ui::ShapeDialog;
use crate::state::Dialog;

/// What an export menu item asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    /// The current frame as a PNG.
    Image,
    /// One of the animation formats.
    Format(Format),
}

/// A picked menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Run a registered command.
    Command {
        id: &'static str,
        params: Value,
    },
    New,
    Open,
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    Import {
        to_stage: bool,
    },
    Export(ExportKind),
    Quit,
    Cut,
    Copy,
    Paste,
    /// Paste at the objects' original coordinates (Ctrl+Shift+V).
    PasteInPlace,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    ToggleOnion,
    ToggleOnionOutlines,
    ToggleRulers,
    ToggleGrid,
    ToggleGuides,
    ToggleTheme,
    OpenPanel(PanelId),
    Play,
    Stop,
    ToggleLoop,
    GoStart,
    GoEnd,
    StepForward,
    StepBack,
    Dialog(Dialog),
    /// A frame operation on the selected frames (Edit > Timeline, Modify > Timeline).
    Frame(FrameAction),
    /// A new layer of this type above the current layer (Insert > Timeline).
    AddLayer(LayerKind),
    /// A motion guide for the current layer.
    AddMotionGuide,
    /// The Layer Properties dialog for the current layer.
    LayerProperties,
    /// Edit the selected symbol instance in place (Edit > Edit in Place).
    EditInPlace,
    /// Edit a library symbol on its own stage (Edit > Edit Symbols).
    EditSymbol,
    ToggleScenePanel,
    /// Opens or closes the Motion Editor (also how the custom ease editor is reached from the Modify menu).
    ToggleMotionEditor,
    ToggleParentingView,
    ToggleEditMultiple,
    ToggleAutoKeyframe,
    /// Tabs, templates, Save All, Find and the other document-level items.
    Doc(DocAction),
}

/// What the menus need to know to enable, disable and check items.
#[derive(Debug, Clone, Default)]
pub struct MenuView {
    pub enabled: Vec<&'static str>,
    pub can_undo: Option<String>,
    pub can_redo: Option<String>,
    pub playing: bool,
    /// A Test Movie is running.
    pub testing: bool,
    pub looping: bool,
    pub onion: bool,
    pub onion_outlines: bool,
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub has_selection: bool,
    pub closed_panels: Vec<PanelId>,
    pub recent: Vec<PathBuf>,
    pub dirty: bool,
    pub scene_panel: bool,
    pub motion_editor: bool,
    pub parenting_view: bool,
    pub edit_multiple: bool,
    pub auto_keyframe: bool,
    /// Open documents.
    pub tabs: usize,
    /// The active document has a file to go back to.
    pub can_revert: bool,
    pub play: anicraft_cli::mcp::docfile::PlayOptions,
    /// Autosave interval in minutes (0 is off).
    pub autosave: u32,
    /// The person's templates: category, name, file.
    pub user_templates: Vec<(String, String, PathBuf)>,
    /// Names of the saved commands.
    pub commands: Vec<String>,
    /// The History, Output and Compiler Errors panels that are open.
    pub open_doc_panels: Vec<PanelId>,
}

impl MenuView {
    fn on(&self, id: &str) -> bool {
        self.enabled.contains(&id)
    }
}

const ALL_COMMANDS: [&str; 26] = [
    "modify.convert_to_symbol",
    "modify.group",
    "modify.ungroup",
    "modify.break_apart",
    "modify.arrange",
    "modify.flip",
    "insert.symbol",
    "layer.add",
    "scene.add",
    "timeline.insert_keyframe",
    "timeline.insert_blank_keyframe",
    "timeline.insert_frames",
    "timeline.remove_frames",
    "timeline.create_classic_tween",
    "timeline.create_shape_tween",
    "timeline.remove_tween",
    "select.all",
    "select.none",
    "edit.delete",
    "edit.duplicate",
    "edit.undo",
    "edit.redo",
    "edit.copy",
    "edit.enter",
    "edit.exit",
    "document.set_stage",
];

/// The ids of commands the menu may enable, for the caller to test against its session.
#[must_use]
pub const fn menu_commands() -> &'static [&'static str] {
    &ALL_COMMANDS
}

fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let mut button = egui::Button::new(crate::text(label));
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    let clicked = ui.add_enabled(enabled, button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn check(ui: &mut Ui, label: &str, shortcut: &str, checked: bool) -> bool {
    let mut button = egui::Button::selectable(checked, crate::text(label));
    if !shortcut.is_empty() {
        button = button.shortcut_text(shortcut);
    }
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn command(id: &'static str) -> MenuAction {
    MenuAction::Command { id, params: json!({}) }
}

/// Draws the menu bar and returns what was picked.
pub fn menu_bar(ui: &mut Ui, view: &MenuView) -> Vec<MenuAction> {
    let mut picked = Vec::new();
    let mut push = |condition: bool, action: MenuAction| {
        if condition {
            picked.push(action);
        }
    };
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            push(item(ui, "New...", "Ctrl+N", true), MenuAction::New);
            ui.menu_button("New from Template", |ui| {
                let mut categories: Vec<&str> = BUILTIN_CATEGORY_ORDER.to_vec();
                for (category, _, _) in &view.user_templates {
                    if !categories.contains(&category.as_str()) {
                        categories.push(category.as_str());
                    }
                }
                for category in categories {
                    ui.menu_button(crate::text(category), |ui| {
                        for t in anicraft_cli::mcp::docfile::BUILTIN_TEMPLATES.iter().filter(|t| t.category == category) {
                            push(item(ui, t.name, "", true), MenuAction::Doc(DocAction::NewFrom(TemplateChoice::Builtin(t.id))));
                        }
                        for (_, name, path) in view.user_templates.iter().filter(|t| t.0 == category) {
                            push(item(ui, name, "", true), MenuAction::Doc(DocAction::NewFrom(TemplateChoice::User(path.clone()))));
                        }
                    });
                }
            });
            push(item(ui, "Open...", "Ctrl+O", true), MenuAction::Open);
            ui.add_enabled_ui(!view.recent.is_empty(), |ui| {
                ui.menu_button("Open Recent", |ui| {
                    for path in &view.recent {
                        let label = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                        let clicked = ui.button(crate::text(&label)).on_hover_text(path.display().to_string()).clicked();
                        push(clicked, MenuAction::OpenRecent(path.clone()));
                        if clicked {
                            ui.close();
                        }
                    }
                    ui.separator();
                    push(item(ui, "Clear Recent Files", "", true), MenuAction::Doc(DocAction::ClearRecent));
                });
            });
            push(item(ui, "Close", "Ctrl+W", true), MenuAction::Doc(DocAction::CloseTab));
            push(item(ui, "Close All", "Ctrl+Shift+W", view.tabs > 1), MenuAction::Doc(DocAction::CloseAll));
            ui.separator();
            push(item(ui, "Save", "Ctrl+S", true), MenuAction::Save);
            push(item(ui, "Save As...", "Ctrl+Shift+S", true), MenuAction::SaveAs);
            push(item(ui, "Save as Template...", "", true), MenuAction::Doc(DocAction::SaveTemplate));
            push(item(ui, "Save All", "", view.dirty || view.tabs > 1), MenuAction::Doc(DocAction::SaveAll));
            push(item(ui, "Save and Compact", "", true), MenuAction::Doc(DocAction::SaveCompact));
            push(item(ui, "Revert", "", view.can_revert), MenuAction::Doc(DocAction::Revert));
            ui.separator();
            ui.menu_button("Import", |ui| {
                push(item(ui, "Import to Stage...", "Ctrl+R", true), MenuAction::Import { to_stage: true });
                push(item(ui, "Import to Library...", "", true), MenuAction::Import { to_stage: false });
            });
            ui.menu_button("Export", |ui| {
                push(item(ui, "Export Image (PNG)...", "", true), MenuAction::Export(ExportKind::Image));
                for (label, format) in [
                    ("Export Animated GIF...", Format::Gif),
                    ("Export Video (MP4)...", Format::Mp4),
                    ("Export Video (MOV, alpha)...", Format::Mov),
                    ("Export SVG (current frame)...", Format::Svg),
                    ("Export Animated SVG...", Format::SvgAnim),
                    ("Export Sprite Sheet...", Format::Spritesheet),
                    ("Export SWF...", Format::Swf),
                    ("Export PNG Sequence...", Format::PngSeq),
                ] {
                    push(item(ui, label, "", true), MenuAction::Export(ExportKind::Format(format)));
                }
            });
            ui.separator();
            push(item(ui, "Document Settings...", "Ctrl+J", true), MenuAction::Doc(DocAction::OpenSettings));
            ui.menu_button("Autosave", |ui| {
                for minutes in crate::prefs::AUTOSAVE_CHOICES {
                    let label = if minutes == 0 { "Off".to_owned() } else { format!("Every {minutes} minute{}", if minutes == 1 { "" } else { "s" }) };
                    push(check(ui, &label, "", view.autosave == minutes), MenuAction::Doc(DocAction::SetAutosave(minutes)));
                }
            });
            ui.separator();
            push(item(ui, "Quit", "Ctrl+Q", true), MenuAction::Quit);
        });
        ui.menu_button("Edit", |ui| {
            let undo = view.can_undo.as_ref().map_or_else(|| "Undo".to_owned(), |l| format!("Undo {l}"));
            let redo = view.can_redo.as_ref().map_or_else(|| "Redo".to_owned(), |l| format!("Redo {l}"));
            push(item(ui, &undo, "Ctrl+Z", view.can_undo.is_some()), command("edit.undo"));
            push(item(ui, &redo, "Ctrl+Shift+Z", view.can_redo.is_some()), command("edit.redo"));
            ui.separator();
            push(item(ui, "Cut", "Ctrl+X", view.has_selection), MenuAction::Cut);
            push(item(ui, "Copy", "Ctrl+C", view.has_selection), MenuAction::Copy);
            push(item(ui, "Paste", "Ctrl+V", true), MenuAction::Paste);
            push(item(ui, "Paste in Place", "Ctrl+Shift+V", true), MenuAction::PasteInPlace);
            push(item(ui, "Paste Special...", "", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::paste_special())));
            push(item(ui, "Delete", "Delete", view.on("edit.delete")), command("edit.delete"));
            push(item(ui, "Duplicate", "Ctrl+D", view.on("edit.duplicate")), command("edit.duplicate"));
            ui.separator();
            push(item(ui, "Select All", "Ctrl+A", true), command("select.all"));
            push(item(ui, "Deselect All", "Ctrl+Shift+A", true), command("select.none"));
            ui.separator();
            ui.menu_button("Timeline", |ui| {
                for (label, shortcut, action) in [
                    ("Cut Frames", "Ctrl+Alt+X", FrameAction::CutFrames),
                    ("Copy Frames", "Ctrl+Alt+C", FrameAction::CopyFrames),
                    ("Paste Frames", "Ctrl+Alt+V", FrameAction::PasteFrames),
                    ("Paste and Overwrite Frames", "", FrameAction::PasteOverwrite),
                    ("Clear Frames", "Alt+Backspace", FrameAction::ClearFrames),
                    ("Select All Frames", "Ctrl+Alt+A", FrameAction::SelectAllFrames),
                ] {
                    push(item(ui, label, shortcut, true), MenuAction::Frame(action));
                }
            });
            ui.separator();
            push(item(ui, "Edit Symbols", "Ctrl+E", true), MenuAction::EditSymbol);
            push(item(ui, "Edit in Place", "", view.has_selection), MenuAction::EditInPlace);
            ui.separator();
            push(item(ui, "Find and Replace...", "Ctrl+F", true), MenuAction::Doc(DocAction::Find));
            push(item(ui, "Find Next", "F3", true), MenuAction::Doc(DocAction::FindAgain));
            ui.separator();
            push(
                check(ui, "History", "Ctrl+F10", view.open_doc_panels.contains(&crate::docs::HISTORY_PANEL)),
                MenuAction::Doc(DocAction::TogglePanel(crate::docs::HISTORY_PANEL)),
            );
        });
        ui.menu_button("Commands", |ui| {
            push(item(ui, "Open History Panel", "", true), MenuAction::Doc(DocAction::TogglePanel(crate::docs::HISTORY_PANEL)));
            ui.separator();
            if view.commands.is_empty() {
                ui.add_enabled(false, egui::Button::new("No saved commands"));
            }
            for name in &view.commands {
                ui.horizontal(|ui| {
                    let run = ui.button(crate::text(name)).clicked();
                    push(run, MenuAction::Doc(DocAction::RunCommand(name.clone())));
                    let delete = ui.small_button("x").on_hover_text("Delete this command").clicked();
                    push(delete, MenuAction::Doc(DocAction::DeleteCommand(name.clone())));
                    if run || delete {
                        ui.close();
                    }
                });
            }
        });
        ui.menu_button("View", |ui| {
            push(item(ui, "Zoom In", "Ctrl+=", true), MenuAction::ZoomIn);
            push(item(ui, "Zoom Out", "Ctrl+-", true), MenuAction::ZoomOut);
            push(item(ui, "Fit in Window", "Ctrl+0", true), MenuAction::ZoomFit);
            push(item(ui, "Actual Size (100%)", "Ctrl+1", true), MenuAction::Zoom100);
            ui.separator();
            push(check(ui, "Onion Skin", "", view.onion), MenuAction::ToggleOnion);
            push(check(ui, "Onion Skin Outlines", "", view.onion_outlines), MenuAction::ToggleOnionOutlines);
            ui.separator();
            push(check(ui, "Rulers", "Ctrl+Shift+R", view.rulers), MenuAction::ToggleRulers);
            push(check(ui, "Grid", "Ctrl+'", view.grid), MenuAction::ToggleGrid);
            push(check(ui, "Guides", "Ctrl+;", view.guides), MenuAction::ToggleGuides);
            ui.separator();
            push(item(ui, "Light / Dark Theme", "", true), MenuAction::ToggleTheme);
            ui.menu_button("Show Panel", |ui| {
                for panel in &view.closed_panels {
                    let clicked = ui.button(panel_title(*panel)).clicked();
                    push(clicked, MenuAction::OpenPanel(*panel));
                    if clicked {
                        ui.close();
                    }
                }
                for panel in DOC_PANELS {
                    let open = view.open_doc_panels.contains(&panel);
                    push(check(ui, &panel_title(panel), "", open), MenuAction::Doc(DocAction::TogglePanel(panel)));
                }
            });
        });
        ui.menu_button("Insert", |ui| {
            push(item(ui, "New Symbol...", "Ctrl+F8", true), MenuAction::Dialog(Dialog::NewSymbol { name: "Symbol 1".to_owned(), kind: 0 }));
            ui.separator();
            ui.menu_button("Timeline", |ui| {
                push(item(ui, "Layer", "", view.on("layer.add")), command("layer.add"));
                push(item(ui, "Layer Folder", "", true), MenuAction::AddLayer(LayerKind::Folder));
                push(item(ui, "Guide Layer", "", true), MenuAction::AddLayer(LayerKind::Guide));
                push(item(ui, "Mask Layer", "", true), MenuAction::AddLayer(LayerKind::Mask));
                push(item(ui, "Classic Motion Guide", "", true), MenuAction::AddMotionGuide);
                ui.separator();
                push(item(ui, "Scene", "", view.on("scene.add")), MenuAction::Command { id: "scene.add", params: json!({ "name": "Scene" }) });
            });
            ui.separator();
            push(item(ui, "Frame", "F5", true), MenuAction::Command { id: "timeline.insert_frames", params: json!({ "count": 1 }) });
            push(item(ui, "Keyframe", "F6", true), command("timeline.insert_keyframe"));
            push(item(ui, "Blank Keyframe", "F7", true), command("timeline.insert_blank_keyframe"));
            ui.separator();
            push(item(ui, "Classic Tween", "", true), command("timeline.create_classic_tween"));
            push(item(ui, "Shape Tween", "", true), command("timeline.create_shape_tween"));
            push(item(ui, "Motion Tween", "", true), command("timeline.create_motion_tween"));
            push(item(ui, "Remove Tween", "", true), command("timeline.remove_tween"));
        });
        ui.menu_button("Modify", |ui| {
            push(
                item(ui, "Convert to Symbol...", "F8", view.has_selection),
                MenuAction::Dialog(Dialog::ConvertToSymbol { name: "Symbol 1".to_owned(), kind: 0 }),
            );
            push(item(ui, "Break Apart", "Ctrl+B", view.has_selection), command("modify.break_apart"));
            ui.separator();
            push(item(ui, "Group", "Ctrl+G", view.has_selection), command("modify.group"));
            push(item(ui, "Ungroup", "Ctrl+Shift+G", view.has_selection), command("modify.ungroup"));
            ui.separator();
            ui.add_enabled_ui(view.has_selection, |ui| {
                ui.menu_button("Arrange", |ui| {
                    for (label, mode) in [("Bring to Front", "front"), ("Bring Forward", "forward"), ("Send Backward", "backward"), ("Send to Back", "back")] {
                        push(item(ui, label, "", true), MenuAction::Command { id: "modify.arrange", params: json!({ "mode": mode }) });
                    }
                    ui.separator();
                    push(item(ui, "Lock", "Ctrl+Alt+L", true), command("modify.lock"));
                });
                ui.menu_button("Transform", |ui| {
                    push(item(ui, "Scale and Rotate...", "Ctrl+Alt+S", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::scale_rotate())));
                    ui.separator();
                    push(
                        item(ui, "Rotate 90\u{b0} CW", "Ctrl+Shift+9", true),
                        MenuAction::Command { id: "modify.transform", params: json!({ "op": "rotate_90_cw" }) },
                    );
                    push(
                        item(ui, "Rotate 90\u{b0} CCW", "Ctrl+Shift+7", true),
                        MenuAction::Command { id: "modify.transform", params: json!({ "op": "rotate_90_ccw" }) },
                    );
                    ui.separator();
                    push(item(ui, "Flip Horizontal", "", true), MenuAction::Command { id: "modify.flip", params: json!({ "axis": "horizontal" }) });
                    push(item(ui, "Flip Vertical", "", true), MenuAction::Command { id: "modify.flip", params: json!({ "axis": "vertical" }) });
                    ui.separator();
                    push(item(ui, "Remove Transform", "", true), MenuAction::Command { id: "modify.transform", params: json!({ "op": "remove" }) });
                });
                ui.menu_button("Combine Objects", |ui| {
                    for (label, op) in [("Union", "union"), ("Intersect", "intersect"), ("Punch", "punch"), ("Crop", "crop")] {
                        push(item(ui, label, "", true), MenuAction::Command { id: "modify.combine_objects", params: json!({ "op": op }) });
                    }
                });
                ui.menu_button("Bitmap", |ui| {
                    push(item(ui, "Convert to Bitmap", "", true), command("modify.convert_to_bitmap"));
                    push(item(ui, "Trace Bitmap...", "", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::trace_bitmap())));
                });
            });
            push(item(ui, "Unlock All", "Ctrl+Alt+Shift+L", true), command("modify.unlock_all"));
            ui.separator();
            ui.menu_button("Shape", |ui| {
                ui.add_enabled_ui(view.has_selection, |ui| {
                    for (label, op) in [("Smooth", "smooth"), ("Straighten", "straighten")] {
                        push(item(ui, label, "", true), MenuAction::Command { id: "modify.shape_curves", params: json!({ "op": op }) });
                    }
                    push(item(ui, "Optimize...", "Ctrl+Alt+Shift+C", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::optimize())));
                    ui.separator();
                    push(item(ui, "Convert Lines to Fills", "", true), command("modify.convert_lines_to_fills"));
                    push(item(ui, "Expand Fill...", "", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::expand_fill())));
                    push(item(ui, "Soften Fill Edges...", "", true), MenuAction::Dialog(Dialog::Shape(ShapeDialog::soften_edges())));
                });
                ui.separator();
                push(item(ui, "Add Shape Hint", "Ctrl+Shift+H", true), command("shape.add_hint"));
                push(item(ui, "Remove All Hints", "", true), command("shape.remove_hint"));
                ui.separator();
                push(item(ui, "Custom Ease...", "", true), MenuAction::ToggleMotionEditor);
            });
            ui.separator();
            ui.menu_button("Timeline", |ui| {
                push(item(ui, "Layer Properties...", "", true), MenuAction::LayerProperties);
                push(item(ui, "Frame Properties...", "", true), MenuAction::Frame(FrameAction::Properties));
                push(item(ui, "Add Frame Label...", "", true), MenuAction::Frame(FrameAction::EditLabel));
                ui.separator();
                push(item(ui, "Reverse Frames", "", true), MenuAction::Frame(FrameAction::ReverseFrames));
                push(item(ui, "Convert to Keyframes", "F6", true), MenuAction::Frame(FrameAction::ConvertToKeyframes));
                push(item(ui, "Convert to Blank Keyframes", "F7", true), MenuAction::Frame(FrameAction::ConvertToBlankKeyframes));
                ui.separator();
                push(item(ui, "Distribute to Layers", "Ctrl+Shift+D", view.has_selection), command("modify.distribute_to_layers"));
                push(item(ui, "Distribute to Keyframes", "", view.has_selection), command("modify.distribute_to_keyframes"));
                ui.separator();
                push(check(ui, "Edit Multiple Frames", "", view.edit_multiple), MenuAction::ToggleEditMultiple);
                push(check(ui, "Auto Keyframe", "", view.auto_keyframe), MenuAction::ToggleAutoKeyframe);
            });
            ui.separator();
            push(item(ui, "Document...", "Ctrl+J", true), MenuAction::Doc(DocAction::OpenSettings));
        });
        ui.menu_button("Control", |ui| {
            push(item(ui, if view.playing { "Pause" } else { "Play" }, "Enter", true), MenuAction::Play);
            push(item(ui, "Stop", "", true), MenuAction::Stop);
            ui.separator();
            push(item(ui, if view.testing { "End Test Movie" } else { "Test Movie" }, "Ctrl+Enter", true), MenuAction::Doc(DocAction::TestMovie));
            ui.separator();
            push(item(ui, "Go to First Frame", "Home", true), MenuAction::GoStart);
            push(item(ui, "Go to Last Frame", "End", true), MenuAction::GoEnd);
            push(item(ui, "Step Forward One Frame", ".", true), MenuAction::StepForward);
            push(item(ui, "Step Backward One Frame", ",", true), MenuAction::StepBack);
            ui.separator();
            push(check(ui, "Loop Playback", "", view.looping), MenuAction::ToggleLoop);
            ui.separator();
            push(check(ui, "Mute Sounds", "Ctrl+Alt+M", view.play.mute_sounds), MenuAction::Doc(DocAction::Play(PlayToggle::MuteSounds)));
            push(check(ui, "Enable Simple Buttons", "Ctrl+Alt+B", view.play.simple_buttons), MenuAction::Doc(DocAction::Play(PlayToggle::SimpleButtons)));
            push(check(ui, "Enable Live Frame Actions", "", view.play.frame_actions), MenuAction::Doc(DocAction::Play(PlayToggle::FrameActions)));
        });
        ui.menu_button("Window", |ui| {
            push(check(ui, "Scene", "Shift+F2", view.scene_panel), MenuAction::ToggleScenePanel);
            push(check(ui, "Motion Editor", "", view.motion_editor), MenuAction::ToggleMotionEditor);
            push(check(ui, "Parenting View", "", view.parenting_view), MenuAction::ToggleParentingView);
        });
        ui.menu_button("Help", |ui| {
            push(item(ui, "Keyboard Shortcuts and About", "F1", true), MenuAction::Dialog(Dialog::About));
        });
    });
    picked
}
