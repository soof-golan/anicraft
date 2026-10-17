//! Layout and snapping: the Align and Transform panels, the View menu's magnification, grid, guides and snapping
//! items, their dialogs, and the stage context menu.
//!
//! Everything here draws from the document's guides and grid and the tool settings and returns [`LayoutEvent`]s;
//! nothing changes the document. The window turns the events into commands (`layout.*`, `transform.apply`,
//! `guides.*`, `grid.set` and `tools.options`).

use anicraft_tools::guides::Layout;
use anicraft_tools::layout::{Axis, Edge};
use anicraft_tools::settings::SnapAccuracy;
use anicraft_tools::{SnapSettings, ToolSettings};
use anim_core::Rgba8;
use anim_ui::PanelId;
use anim_ui::stage::{Axis as GuideAxis, Guide, StageViewState};
use egui::{Color32, Context, DragValue, Pos2, Ui};
use serde_json::{Value, json};

use crate::menus::MenuAction;
use crate::state::Dialog;

/// How far the stage texture reaches beyond the stage when the pasteboard is shown, as a fraction of the stage size
/// on every side.
pub const PASTEBOARD_MARGIN: f32 = 0.5;

/// Panel id of the Align panel (a `PanelId::Custom`).
pub const ALIGN_PANEL: u16 = 3101;
/// Panel id of the Transform panel (a `PanelId::Custom`).
pub const TRANSFORM_PANEL: u16 = 3102;

/// The tab title of one of this module's panels.
#[must_use]
pub fn panel_title(panel: PanelId) -> Option<String> {
    match panel {
        PanelId::Custom(ALIGN_PANEL) => Some("Align".to_owned()),
        PanelId::Custom(TRANSFORM_PANEL) => Some("Transform".to_owned()),
        _ => None,
    }
}

/// Opens the Align and Transform panels as tabs beside the Properties panel.
pub fn open_panels(layout: &mut anim_ui::dock::docking::Layout<PanelId>) {
    for id in [ALIGN_PANEL, TRANSFORM_PANEL] {
        let _ = anim_ui::dock::open_panel(layout, PanelId::Custom(id));
    }
}

/// Copies the document's guides and grid into the stage widget's state.
pub fn sync_stage(stage: &mut StageViewState, aids: &Layout) {
    let paint = |c: Rgba8| Color32::from_rgb(c.r, c.g, c.b);
    stage.show_grid = aids.grid_visible;
    stage.show_guides = aids.guides_visible;
    stage.lock_guides = aids.guides_locked;
    stage.grid_size = anim_core::num::f64_to_f32(aids.grid[0]);
    stage.grid_size_y = anim_core::num::f64_to_f32(aids.grid[1]);
    stage.grid_color = Some(paint(aids.grid_color));
    stage.guide_color = Some(paint(aids.guide_color));
    stage.guides = aids
        .guides_x
        .iter()
        .map(|x| Guide { axis: GuideAxis::Vertical, pos: anim_core::num::f64_to_f32(*x) })
        .chain(aids.guides_y.iter().map(|y| Guide { axis: GuideAxis::Horizontal, pos: anim_core::num::f64_to_f32(*y) }))
        .collect();
}

/// The name `guides.*` commands know a guide direction by.
#[must_use]
pub const fn axis_name(axis: GuideAxis) -> &'static str {
    match axis {
        GuideAxis::Horizontal => "horizontal",
        GuideAxis::Vertical => "vertical",
    }
}

/// Where the guide at `index` of the stage's guide list sits among the guides of its own direction.
#[must_use]
pub fn guide_slot(guides: &[Guide], index: usize) -> Option<(&'static str, usize)> {
    let guide = guides.get(index)?;
    let before = guides.iter().take(index).filter(|g| g.axis == guide.axis).count();
    Some((axis_name(guide.axis), before))
}

// ---- events -------------------------------------------------------------------------------------------------------

/// The dialogs of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Grid,
    Guides,
    Snapping,
}

/// What the panels, menus and dialogs of this module ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutEvent {
    /// Run a registered command.
    Command { id: &'static str, params: Value },
    /// Zoom the stage to a percentage.
    Magnify(u32),
    /// View > Magnification > Show Frame.
    ShowFrame,
    /// View > Magnification > Show All.
    ShowAll,
    /// View > Pasteboard.
    TogglePasteboard,
    /// The Align panel's "Align to stage" check box.
    ToggleToStage,
    /// Open or close one of the panels.
    TogglePanel(u16),
    /// Open a dialog with the current values.
    Open(DialogKind),
    /// Close the dialog without applying anything.
    Close,
}

impl LayoutEvent {
    fn command(id: &'static str, params: Value) -> Self {
        Self::Command { id, params }
    }

    fn options(patch: Value) -> Self {
        Self::command("tools.options", patch)
    }
}

/// A [`LayoutEvent`] as a menu action.
fn action(event: LayoutEvent) -> MenuAction {
    MenuAction::Layout(event)
}

// ---- state --------------------------------------------------------------------------------------------------------

/// The Transform panel's fields.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformFields {
    /// Scale factors in percent of the current size.
    pub scale: [f64; 2],
    /// Keep the proportions.
    pub constrain: bool,
    pub rotate: f64,
    /// Skew angles in degrees (horizontal, vertical).
    pub skew: [f64; 2],
    /// 3D rotation about the x, y and z axes in degrees, applied to movie clips.
    pub rotate_3d: [f64; 3],
    /// 3D translation along x, y and z in pixels.
    pub move_3d: [f64; 3],
}

impl Default for TransformFields {
    fn default() -> Self {
        Self { scale: [100.0, 100.0], constrain: true, rotate: 0.0, skew: [0.0, 0.0], rotate_3d: [0.0; 3], move_3d: [0.0; 3] }
    }
}

/// The values a dialog edits, copied from the document when it opens.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutDialog {
    Grid { visible: bool, snap: bool, spacing: [f64; 2], link: bool, color: [u8; 3], accuracy: SnapAccuracy },
    Guides { visible: bool, locked: bool, snap: bool, color: [u8; 3], accuracy: SnapAccuracy },
    Snapping { snap: SnapSettings },
}

impl LayoutDialog {
    /// The dialog of `kind` filled from the document's layout and the snapping switches.
    #[must_use]
    pub fn open(kind: DialogKind, aids: &Layout, settings: &ToolSettings) -> Self {
        let rgb = |c: Rgba8| [c.r, c.g, c.b];
        match kind {
            DialogKind::Grid => Self::Grid {
                visible: aids.grid_visible,
                snap: settings.snap.grid,
                spacing: aids.grid,
                link: (aids.grid[0] - aids.grid[1]).abs() < f64::EPSILON,
                color: rgb(aids.grid_color),
                accuracy: settings.snap.accuracy,
            },
            DialogKind::Guides => Self::Guides {
                visible: aids.guides_visible,
                locked: aids.guides_locked,
                snap: settings.snap.guides,
                color: rgb(aids.guide_color),
                accuracy: settings.snap.accuracy,
            },
            DialogKind::Snapping => Self::Snapping { snap: settings.snap.clone() },
        }
    }
}

/// An open stage context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenu {
    /// Where the menu opens on screen; set when it is first drawn.
    pub at: Option<Pos2>,
}

/// View state of this module.
#[derive(Debug, Clone)]
pub struct LayoutUi {
    /// The Align panel acts relative to the stage.
    pub to_stage: bool,
    /// Content outside the stage is shown on the pasteboard.
    pub pasteboard: bool,
    pub transform: TransformFields,
    pub dialog: Option<LayoutDialog>,
    pub context: Option<ContextMenu>,
}

impl Default for LayoutUi {
    fn default() -> Self {
        Self { to_stage: false, pasteboard: true, transform: TransformFields::default(), dialog: None, context: None }
    }
}

/// What the View and Modify menus need to know.
#[derive(Debug, Clone, Default)]
pub struct MenuState {
    pub aids: Layout,
    pub snap: SnapSettings,
    pub pasteboard: bool,
    pub align_open: bool,
    pub transform_open: bool,
    pub has_selection: bool,
}

// ---- menus --------------------------------------------------------------------------------------------------------

/// The magnification presets of the View menu, in percent.
pub const PRESETS: [u32; 7] = [25, 50, 100, 200, 400, 800, 1600];

const EDGES: [(&str, Edge); 6] = [
    ("Left", Edge::Left),
    ("Horizontal Center", Edge::HorizontalCenter),
    ("Right", Edge::Right),
    ("Top", Edge::Top),
    ("Vertical Center", Edge::VerticalCenter),
    ("Bottom", Edge::Bottom),
];

fn edge_name(edge: Edge) -> &'static str {
    match edge {
        Edge::Left => "left",
        Edge::HorizontalCenter => "horizontal_center",
        Edge::Right => "right",
        Edge::Top => "top",
        Edge::VerticalCenter => "vertical_center",
        Edge::Bottom => "bottom",
    }
}

fn axis_word(axis: Axis) -> &'static str {
    match axis {
        Axis::Horizontal => "horizontal",
        Axis::Vertical => "vertical",
    }
}

fn entry(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
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

fn tick(ui: &mut Ui, label: &str, shortcut: &str, checked: bool) -> bool {
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

/// View > Magnification, Pasteboard, Grid, Guides and Snapping.
#[must_use]
pub fn view_items(ui: &mut Ui, state: &MenuState) -> Vec<MenuAction> {
    let mut out = Vec::new();
    let mut push = |condition: bool, event: LayoutEvent| {
        if condition {
            out.push(action(event));
        }
    };
    ui.menu_button("Magnification", |ui| {
        push(entry(ui, "Show Frame", "Ctrl+0", true), LayoutEvent::ShowFrame);
        push(entry(ui, "Show All", "Ctrl+Alt+0", true), LayoutEvent::ShowAll);
        ui.separator();
        for percent in PRESETS {
            push(entry(ui, &format!("{percent}%"), "", true), LayoutEvent::Magnify(percent));
        }
    });
    push(tick(ui, "Pasteboard", "", state.pasteboard), LayoutEvent::TogglePasteboard);
    ui.separator();
    ui.menu_button("Grid", |ui| {
        push(tick(ui, "Show Grid", "Ctrl+'", state.aids.grid_visible), LayoutEvent::command("grid.set", json!({ "visible": !state.aids.grid_visible })));
        push(tick(ui, "Snap to Grid", "Ctrl+Shift+'", state.snap.grid), LayoutEvent::options(json!({ "snap": { "grid": !state.snap.grid } })));
        ui.separator();
        push(entry(ui, "Edit Grid...", "Ctrl+Alt+G", true), LayoutEvent::Open(DialogKind::Grid));
    });
    ui.menu_button("Guides", |ui| {
        push(
            tick(ui, "Show Guides", "Ctrl+;", state.aids.guides_visible),
            LayoutEvent::command("guides.set", json!({ "visible": !state.aids.guides_visible })),
        );
        push(
            tick(ui, "Lock Guides", "Ctrl+Alt+;", state.aids.guides_locked),
            LayoutEvent::command("guides.set", json!({ "locked": !state.aids.guides_locked })),
        );
        push(tick(ui, "Snap to Guides", "Ctrl+Shift+;", state.snap.guides), LayoutEvent::options(json!({ "snap": { "guides": !state.snap.guides } })));
        ui.separator();
        push(entry(ui, "Edit Guides...", "Ctrl+Alt+Shift+G", true), LayoutEvent::Open(DialogKind::Guides));
        let any = !(state.aids.guides_x.is_empty() && state.aids.guides_y.is_empty());
        push(entry(ui, "Clear Guides", "", any), LayoutEvent::command("guides.clear", json!({})));
    });
    ui.menu_button("Snapping", |ui| {
        push(tick(ui, "Snap Align", "", state.snap.align), LayoutEvent::options(json!({ "snap": { "align": !state.snap.align } })));
        push(tick(ui, "Snap to Grid", "Ctrl+Shift+'", state.snap.grid), LayoutEvent::options(json!({ "snap": { "grid": !state.snap.grid } })));
        push(tick(ui, "Snap to Guides", "Ctrl+Shift+;", state.snap.guides), LayoutEvent::options(json!({ "snap": { "guides": !state.snap.guides } })));
        push(tick(ui, "Snap to Pixels", "", state.snap.pixels), LayoutEvent::options(json!({ "snap": { "pixels": !state.snap.pixels } })));
        push(tick(ui, "Snap to Objects", "Ctrl+Shift+/", state.snap.objects), LayoutEvent::options(json!({ "snap": { "objects": !state.snap.objects } })));
        push(tick(ui, "Show Smart Guides", "", state.snap.smart_guides), LayoutEvent::options(json!({ "snap": { "smart_guides": !state.snap.smart_guides } })));
        ui.separator();
        push(entry(ui, "Edit Snapping...", "", true), LayoutEvent::Open(DialogKind::Snapping));
    });
    out
}

/// Modify > Align: the Align panel's buttons as menu items.
#[must_use]
pub fn align_items(ui: &mut Ui, state: &MenuState, to_stage: bool) -> Vec<MenuAction> {
    let mut out = Vec::new();
    let mut push = |condition: bool, event: LayoutEvent| {
        if condition {
            out.push(action(event));
        }
    };
    ui.add_enabled_ui(state.has_selection, |ui| {
        ui.menu_button("Align", |ui| {
            for (label, edge) in EDGES {
                push(entry(ui, label, "", true), LayoutEvent::command("layout.align", json!({ "edge": edge_name(edge), "to_stage": to_stage })));
            }
            ui.separator();
            ui.menu_button("Distribute", |ui| {
                for (label, edge) in EDGES {
                    push(entry(ui, label, "", true), LayoutEvent::command("layout.distribute", json!({ "edge": edge_name(edge), "to_stage": to_stage })));
                }
            });
            ui.menu_button("Match Size", |ui| {
                for (label, width, height) in [("Width", true, false), ("Height", false, true), ("Width and Height", true, true)] {
                    push(
                        entry(ui, label, "", true),
                        LayoutEvent::command("layout.match_size", json!({ "width": width, "height": height, "to_stage": to_stage })),
                    );
                }
            });
            ui.menu_button("Space Evenly", |ui| {
                for (label, axis) in [("Horizontally", Axis::Horizontal), ("Vertically", Axis::Vertical)] {
                    push(entry(ui, label, "", true), LayoutEvent::command("layout.space", json!({ "axis": axis_word(axis), "to_stage": to_stage })));
                }
            });
        });
    });
    out
}

/// Window > Align and Transform.
#[must_use]
pub fn window_items(ui: &mut Ui, state: &MenuState) -> Vec<MenuAction> {
    let mut out = Vec::new();
    if tick(ui, "Align", "Ctrl+K", state.align_open) {
        out.push(action(LayoutEvent::TogglePanel(ALIGN_PANEL)));
    }
    if tick(ui, "Transform", "Ctrl+T", state.transform_open) {
        out.push(action(LayoutEvent::TogglePanel(TRANSFORM_PANEL)));
    }
    out
}

// ---- panels -------------------------------------------------------------------------------------------------------

fn edge_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(crate::text(label)).min_size(egui::vec2(0.0, 22.0))).clicked()
}

/// The Align panel: align, distribute, match size and space, relative to the selection or the stage.
#[must_use]
pub fn align_panel(ui: &mut Ui, ui_state: &LayoutUi, selected: usize) -> Vec<LayoutEvent> {
    let mut out = Vec::new();
    let to_stage = ui_state.to_stage;
    let enough = selected >= 1;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.strong("Align");
        ui.horizontal_wrapped(|ui| {
            for (label, edge) in EDGES {
                if edge_button(ui, label, enough && (to_stage || selected >= 2)) {
                    out.push(LayoutEvent::command("layout.align", json!({ "edge": edge_name(edge), "to_stage": to_stage })));
                }
            }
        });
        ui.add_space(6.0);
        ui.strong("Distribute");
        ui.horizontal_wrapped(|ui| {
            for (label, edge) in EDGES {
                if edge_button(ui, label, selected >= if to_stage { 2 } else { 3 }) {
                    out.push(LayoutEvent::command("layout.distribute", json!({ "edge": edge_name(edge), "to_stage": to_stage })));
                }
            }
        });
        ui.add_space(6.0);
        ui.strong("Match Size");
        ui.horizontal(|ui| {
            for (label, width, height) in [("Width", true, false), ("Height", false, true), ("Both", true, true)] {
                if edge_button(ui, label, if to_stage { selected >= 1 } else { selected >= 2 }) {
                    out.push(LayoutEvent::command("layout.match_size", json!({ "width": width, "height": height, "to_stage": to_stage })));
                }
            }
        });
        ui.add_space(6.0);
        ui.strong("Space");
        ui.horizontal(|ui| {
            for (label, axis) in [("Horizontally", Axis::Horizontal), ("Vertically", Axis::Vertical)] {
                if edge_button(ui, label, selected >= 2) {
                    out.push(LayoutEvent::command("layout.space", json!({ "axis": axis_word(axis), "to_stage": to_stage })));
                }
            }
        });
        ui.add_space(8.0);
        let mut stage = to_stage;
        if ui.checkbox(&mut stage, "Align to Stage").changed() {
            out.push(LayoutEvent::ToggleToStage);
        }
        if selected == 0 {
            ui.weak("Select two or more objects on the stage.");
        }
    });
    out
}

fn field(ui: &mut Ui, value: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> bool {
    ui.add(DragValue::new(value).range(range).speed(0.5).suffix(suffix)).changed()
}

/// The Transform panel: edits `fields` and returns the commands its buttons ask for. `can_3d` says whether the
/// selection holds a symbol instance that can be turned in 3D.
#[must_use]
pub fn transform_panel(ui: &mut Ui, fields: &mut TransformFields, selected: usize, can_3d: bool) -> Vec<LayoutEvent> {
    let mut out = Vec::new();
    let enabled = selected > 0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.add_enabled_ui(enabled, |ui| {
            egui::Grid::new("transform-fields").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Scale");
                let [mut sx, mut sy] = fields.scale;
                let constrain = fields.constrain;
                let changed_x = field(ui, &mut sx, 1.0..=10_000.0, " %");
                let changed_y = field(ui, &mut sy, 1.0..=10_000.0, " %");
                if changed_x {
                    fields.scale = if constrain { [sx, sx] } else { [sx, fields.scale[1]] };
                } else if changed_y {
                    fields.scale = if constrain { [sy, sy] } else { [fields.scale[0], sy] };
                }
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut fields.constrain, "Constrain");
                ui.end_row();
                ui.label("Rotate");
                field(ui, &mut fields.rotate, -3600.0..=3600.0, " deg");
                ui.end_row();
                ui.label("Skew");
                field(ui, &mut fields.skew[0], -85.0..=85.0, " deg H");
                field(ui, &mut fields.skew[1], -85.0..=85.0, " deg V");
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Apply").on_hover_text("Transform the selection about its centre").clicked() {
                    out.push(apply_event(fields, false));
                }
                if ui.button("Copy and Apply").on_hover_text("Duplicate the selection and transform the copy").clicked() {
                    out.push(apply_event(fields, true));
                }
                if ui.button("Reset").clicked() {
                    *fields = TransformFields::default();
                }
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_enabled_ui(can_3d, |ui| {
            ui.strong("3D Rotation");
            egui::Grid::new("transform-3d").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                for (axis, value) in ["X", "Y", "Z"].into_iter().zip(fields.rotate_3d.iter_mut()) {
                    ui.label(axis);
                    field(ui, value, -3600.0..=3600.0, " deg");
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.strong("3D Translation");
            egui::Grid::new("transform-3d-move").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                for (axis, value) in ["X", "Y", "Z"].into_iter().zip(fields.move_3d.iter_mut()) {
                    ui.label(axis);
                    field(ui, value, -100_000.0..=100_000.0, " px");
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            if ui.button("Apply 3D").on_hover_text("Turn and move the selected movie clips in 3D about their centre points").clicked() {
                out.extend(apply_3d_events(fields));
                fields.rotate_3d = [0.0; 3];
                fields.move_3d = [0.0; 3];
            }
        });
        if !can_3d {
            ui.weak("Select a movie clip to turn it in 3D.");
        }
        if !enabled {
            ui.weak("Select an object on the stage.");
        }
    });
    out
}

/// The 3D commands for the non-zero 3D fields: the rotation first, then the translation.
fn apply_3d_events(fields: &TransformFields) -> Vec<LayoutEvent> {
    let mut out = Vec::new();
    let [rx, ry, rz] = fields.rotate_3d;
    if rx != 0.0 || ry != 0.0 || rz != 0.0 {
        out.push(LayoutEvent::command("transform.rotate_3d", json!({ "x_degrees": rx, "y_degrees": ry, "z_degrees": rz })));
    }
    let [x, y, z] = fields.move_3d;
    if x != 0.0 || y != 0.0 || z != 0.0 {
        out.push(LayoutEvent::command("transform.translate_3d", json!({ "x": x, "y": y, "z": z })));
    }
    out
}

fn apply_event(fields: &TransformFields, copy: bool) -> LayoutEvent {
    LayoutEvent::command(
        "transform.apply",
        json!({
            "sx": fields.scale[0] / 100.0,
            "sy": fields.scale[1] / 100.0,
            "rotate_degrees": fields.rotate,
            "skew_x_degrees": fields.skew[0],
            "skew_y_degrees": fields.skew[1],
            "copy": copy,
        }),
    )
}

// ---- dialogs ------------------------------------------------------------------------------------------------------

fn accuracy_name(a: SnapAccuracy) -> &'static str {
    match a {
        SnapAccuracy::Close => "close",
        SnapAccuracy::Normal => "normal",
        SnapAccuracy::Distant => "distant",
    }
}

fn accuracy_choice(ui: &mut Ui, accuracy: &mut SnapAccuracy) {
    egui::ComboBox::from_id_salt("snap-accuracy")
        .selected_text(match accuracy {
            SnapAccuracy::Close => "Must be close",
            SnapAccuracy::Normal => "Normal",
            SnapAccuracy::Distant => "Can be distant",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(accuracy, SnapAccuracy::Close, "Must be close");
            ui.selectable_value(accuracy, SnapAccuracy::Normal, "Normal");
            ui.selectable_value(accuracy, SnapAccuracy::Distant, "Can be distant");
        });
}

fn hex(c: [u8; 3]) -> String {
    Rgba8::opaque(c[0], c[1], c[2]).to_hex_rgb()
}

/// Draws the open dialog; `OK` yields the commands that apply it, `Cancel` closes it.
pub fn dialog(ctx: &Context, ui_state: &mut LayoutUi) -> Vec<LayoutEvent> {
    let mut out = Vec::new();
    let Some(mut dialog) = ui_state.dialog.take() else { return out };
    let title = match &dialog {
        LayoutDialog::Grid { .. } => "Grid",
        LayoutDialog::Guides { .. } => "Guides",
        LayoutDialog::Snapping { .. } => "Snapping",
    };
    let mut done = None;
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        match &mut dialog {
            LayoutDialog::Grid { visible, snap, spacing, link, color, accuracy } => {
                egui::Grid::new("grid-dialog").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Color");
                    ui.color_edit_button_srgb(color);
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(visible, "Show grid");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(snap, "Snap to grid");
                    ui.end_row();
                    ui.label("Horizontal");
                    if ui.add(DragValue::new(&mut spacing[0]).range(1.0..=1000.0).suffix(" px")).changed() && *link {
                        spacing[1] = spacing[0];
                    }
                    ui.end_row();
                    ui.label("Vertical");
                    if ui.add(DragValue::new(&mut spacing[1]).range(1.0..=1000.0).suffix(" px")).changed() && *link {
                        spacing[0] = spacing[1];
                    }
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(link, "Keep both spacings equal");
                    ui.end_row();
                    ui.label("Snap accuracy");
                    accuracy_choice(ui, accuracy);
                    ui.end_row();
                });
            }
            LayoutDialog::Guides { visible, locked, snap, color, accuracy } => {
                egui::Grid::new("guides-dialog").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Color");
                    ui.color_edit_button_srgb(color);
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(visible, "Show guides");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(snap, "Snap to guides");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(locked, "Lock guides");
                    ui.end_row();
                    ui.label("Snap accuracy");
                    accuracy_choice(ui, accuracy);
                    ui.end_row();
                });
                if ui.button("Clear All").clicked() {
                    out.push(LayoutEvent::command("guides.clear", json!({})));
                }
            }
            LayoutDialog::Snapping { snap } => {
                egui::Grid::new("snap-dialog").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("");
                    ui.checkbox(&mut snap.align, "Snap align");
                    ui.end_row();
                    ui.add_enabled_ui(snap.align, |ui| {
                        ui.label("Distance from edge");
                    });
                    ui.add_enabled_ui(snap.align, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(DragValue::new(&mut snap.edge_margin[0]).range(0.0..=1000.0).suffix(" px H"));
                            ui.add(DragValue::new(&mut snap.edge_margin[1]).range(0.0..=1000.0).suffix(" px V"));
                        });
                    });
                    ui.end_row();
                    ui.add_enabled_ui(snap.align, |ui| {
                        ui.label("Between objects");
                    });
                    ui.add_enabled_ui(snap.align, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(DragValue::new(&mut snap.object_spacing[0]).range(0.0..=1000.0).suffix(" px H"));
                            ui.add(DragValue::new(&mut snap.object_spacing[1]).range(0.0..=1000.0).suffix(" px V"));
                        });
                    });
                    ui.end_row();
                    ui.label("");
                    ui.add_enabled(snap.align, egui::Checkbox::new(&mut snap.center_align, "Center alignment"));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut snap.smart_guides, "Show smart guides");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut snap.objects, "Snap to objects");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut snap.pixels, "Snap to pixels");
                    ui.end_row();
                    ui.label("Snap accuracy");
                    accuracy_choice(ui, &mut snap.accuracy);
                    ui.end_row();
                });
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() {
                done = Some(false);
            }
        });
    });
    match done {
        Some(true) => out.extend(apply_dialog(&dialog)),
        Some(false) => {}
        None => ui_state.dialog = Some(dialog),
    }
    out
}

/// The commands that make a dialog's values take effect.
#[must_use]
pub fn apply_dialog(dialog: &LayoutDialog) -> Vec<LayoutEvent> {
    match dialog {
        LayoutDialog::Grid { visible, snap, spacing, color, accuracy, .. } => vec![
            LayoutEvent::command("grid.set", json!({ "visible": visible, "spacing_x": spacing[0], "spacing_y": spacing[1], "color": hex(*color) })),
            LayoutEvent::options(json!({ "snap": { "grid": snap, "accuracy": accuracy_name(*accuracy) } })),
        ],
        LayoutDialog::Guides { visible, locked, snap, color, accuracy } => vec![
            LayoutEvent::command("guides.set", json!({ "visible": visible, "locked": locked, "color": hex(*color) })),
            LayoutEvent::options(json!({ "snap": { "guides": snap, "accuracy": accuracy_name(*accuracy) } })),
        ],
        LayoutDialog::Snapping { snap } => vec![LayoutEvent::options(json!({
            "snap": {
                "align": snap.align,
                "edge_margin": snap.edge_margin,
                "object_spacing": snap.object_spacing,
                "center_align": snap.center_align,
                "smart_guides": snap.smart_guides,
                "objects": snap.objects,
                "pixels": snap.pixels,
                "accuracy": accuracy_name(snap.accuracy),
            }
        }))],
    }
}

// ---- context menu -------------------------------------------------------------------------------------------------

/// The stage's context menu; returns the picked actions and closes itself when one is picked or the pointer goes
/// elsewhere.
pub fn stage_context_menu(ctx: &Context, ui_state: &mut LayoutUi, state: &MenuState, to_stage: bool) -> Vec<MenuAction> {
    let mut out = Vec::new();
    let Some(menu) = ui_state.context.as_mut() else { return out };
    let at = *menu.at.get_or_insert_with(|| ctx.input(|i| i.pointer.latest_pos()).unwrap_or(Pos2::new(200.0, 200.0)));
    egui::Area::new(egui::Id::new("stage-context")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(190.0);
            let sel = state.has_selection;
            let pick = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, a: MenuAction, out: &mut Vec<MenuAction>| {
                if ui.add_enabled(enabled, egui::Button::new(crate::text(label)).shortcut_text(shortcut)).clicked() {
                    out.push(a);
                }
            };
            let cmd = |id: &'static str, params: Value| MenuAction::Command { id, params };
            pick(ui, "Cut", "Ctrl+X", sel, MenuAction::Cut, &mut out);
            pick(ui, "Copy", "Ctrl+C", sel, MenuAction::Copy, &mut out);
            pick(ui, "Paste", "Ctrl+V", true, MenuAction::Paste, &mut out);
            pick(ui, "Delete", "Delete", sel, cmd("edit.delete", json!({})), &mut out);
            pick(ui, "Duplicate", "Ctrl+D", sel, cmd("edit.duplicate", json!({})), &mut out);
            ui.separator();
            pick(ui, "Select All", "Ctrl+A", true, cmd("select.all", json!({})), &mut out);
            pick(ui, "Deselect All", "Ctrl+Shift+A", sel, cmd("select.none", json!({})), &mut out);
            ui.separator();
            pick(ui, "Group", "Ctrl+G", sel, cmd("modify.group", json!({})), &mut out);
            pick(ui, "Ungroup", "Ctrl+Shift+G", sel, cmd("modify.ungroup", json!({})), &mut out);
            pick(ui, "Break Apart", "Ctrl+B", sel, cmd("modify.break_apart", json!({})), &mut out);
            pick(ui, "Convert to Symbol...", "F8", sel, MenuAction::Dialog(Dialog::ConvertToSymbol { name: "Symbol 1".to_owned(), kind: 0 }), &mut out);
            ui.separator();
            ui.add_enabled_ui(sel, |ui| {
                ui.menu_button("Arrange", |ui| {
                    for (label, mode) in [("Bring to Front", "front"), ("Bring Forward", "forward"), ("Send Backward", "backward"), ("Send to Back", "back")] {
                        if ui.button(label).clicked() {
                            out.push(cmd("modify.arrange", json!({ "mode": mode })));
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Align", |ui| {
                    for (label, edge) in EDGES {
                        if ui.button(label).clicked() {
                            out.push(action(LayoutEvent::command("layout.align", json!({ "edge": edge_name(edge), "to_stage": to_stage }))));
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            pick(ui, "Show Grid", "Ctrl+'", true, action(LayoutEvent::command("grid.set", json!({ "visible": !state.aids.grid_visible }))), &mut out);
            pick(ui, "Show Guides", "Ctrl+;", true, action(LayoutEvent::command("guides.set", json!({ "visible": !state.aids.guides_visible }))), &mut out);
            pick(ui, "Lock Guides", "", true, action(LayoutEvent::command("guides.set", json!({ "locked": !state.aids.guides_locked }))), &mut out);
        });
    });
    // A press on the menu or on one of its submenu popups (all foreground layers) keeps it; a press elsewhere closes it.
    let over_menu = ctx.pointer_latest_pos().and_then(|p| ctx.layer_id_at(p)).is_some_and(|layer| layer.order == egui::Order::Foreground);
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !over_menu;
    if !out.is_empty() || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui_state.context = None;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guide_is_found_among_the_guides_of_its_own_direction() {
        let aids = Layout { guides_x: vec![10.0, 20.0], guides_y: vec![5.0, 6.0, 7.0], ..Layout::default() };
        let mut stage = StageViewState::new(egui::vec2(100.0, 100.0));
        sync_stage(&mut stage, &aids);
        assert_eq!(stage.guides.len(), 5);
        assert_eq!(guide_slot(&stage.guides, 1), Some(("vertical", 1)));
        assert_eq!(guide_slot(&stage.guides, 3), Some(("horizontal", 1)));
        assert_eq!(guide_slot(&stage.guides, 9), None);
    }

    #[test]
    fn the_transform_panel_sends_factors_not_percentages() {
        let fields = TransformFields { scale: [150.0, 50.0], rotate: 30.0, skew: [10.0, 0.0], constrain: false, ..TransformFields::default() };
        let LayoutEvent::Command { id, params } = apply_event(&fields, true) else { panic!("a command") };
        assert_eq!(id, "transform.apply");
        assert_eq!(params["sx"], 1.5);
        assert_eq!(params["sy"], 0.5);
        assert_eq!(params["copy"], true);
    }

    #[test]
    fn the_3d_fields_send_only_what_was_changed() {
        let mut fields = TransformFields::default();
        assert!(apply_3d_events(&fields).is_empty());
        fields.rotate_3d = [0.0, 45.0, 0.0];
        let events = apply_3d_events(&fields);
        assert_eq!(events.len(), 1);
        let LayoutEvent::Command { id, params } = &events[0] else { panic!("a command") };
        assert_eq!(*id, "transform.rotate_3d");
        assert_eq!(params["y_degrees"], 45.0);
        fields.move_3d = [0.0, 0.0, -20.0];
        let events = apply_3d_events(&fields);
        assert_eq!(events.len(), 2);
        let LayoutEvent::Command { id, params } = &events[1] else { panic!("a command") };
        assert_eq!(*id, "transform.translate_3d");
        assert_eq!(params["z"], -20.0);
    }

    #[test]
    fn dialogs_apply_through_registered_commands() {
        let settings = ToolSettings::default();
        let mut dialog = LayoutDialog::open(DialogKind::Grid, &Layout::default(), &settings);
        if let LayoutDialog::Grid { spacing, snap, .. } = &mut dialog {
            *spacing = [30.0, 12.0];
            *snap = true;
        }
        let events = apply_dialog(&dialog);
        assert_eq!(events.len(), 2);
        let LayoutEvent::Command { id, params } = &events[0] else { panic!("a command") };
        assert_eq!(*id, "grid.set");
        assert_eq!(params["spacing_y"], 12.0);
        let LayoutEvent::Command { params, .. } = &events[1] else { panic!("a command") };
        assert_eq!(params["snap"]["grid"], true);
    }
}
