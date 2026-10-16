//! The toolbox, the tool options, the Colour panel and the Swatches panel.
//!
//! Every function here draws from the tool settings and the document's swatches and returns events; nothing changes
//! the document. The window turns the events into commands (`tools.options`, `colour.apply`, `swatches.*`).

#![allow(
    clippy::suboptimal_flops,
    clippy::while_float,
    clippy::type_complexity,
    clippy::large_enum_variant,
    reason = "pixel geometry for hand-drawn icons and chips; short-lived UI events"
)]

mod colour;
mod options;
mod swatches;
mod toolbox;

use std::collections::HashMap;

use anicraft_tools::swatches::TableFormat;
use anim_core::Rgba8;
use anim_shape::{FillStyle, StrokeStyle};
use anim_ui::{Icon, PanelId, ToolItem};
use serde_json::Value;

pub use colour::{FillKind, colour_panel, convert_fill, kind_of};
pub use options::tool_options;
pub use swatches::swatches_panel;
pub use toolbox::toolbox;

/// Panel id of the Colour panel (a `PanelId::Custom`).
pub const COLOUR_PANEL: u16 = 3001;
/// Panel id of the Swatches panel (a `PanelId::Custom`).
pub const SWATCHES_PANEL: u16 = 3002;

/// The tab title of a panel, including the custom ones of this module.
#[must_use]
pub fn panel_title(panel: PanelId) -> String {
    match panel {
        PanelId::Custom(COLOUR_PANEL) => "Colour".to_owned(),
        PanelId::Custom(SWATCHES_PANEL) => "Swatches".to_owned(),
        other => other.title(),
    }
}

/// Opens the Colour and Swatches panels as tabs beside the Properties panel.
pub fn open_colour_panels(layout: &mut anim_ui::dock::docking::Layout<PanelId>) {
    for id in [COLOUR_PANEL, SWATCHES_PANEL] {
        let _ = anim_ui::dock::open_panel(layout, PanelId::Custom(id));
    }
}

/// Which of the two toolbox colours the Colour panel edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColourTarget {
    #[default]
    Fill,
    Stroke,
}

/// A change to the fill and/or the stroke style from the Colour panel or the swatches. With neither set it only
/// ends the drag that was in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct ColourEdit {
    pub fill: Option<FillStyle>,
    pub stroke: Option<StrokeStyle>,
    /// The gesture (a slider drag, a typed value) is finished: the next edit is a new undo step.
    pub commit: bool,
}

/// What the toolbox and the panels report.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolUiEvent {
    /// A tool was picked (toolbar id).
    Tool(String),
    /// Run a registered command (`tools.options`, `colour.swap`, `swatches.use`, ...).
    Command { id: &'static str, params: Value },
    /// Change the fill or stroke style.
    Colour(ColourEdit),
    /// Load a `.aco` or `.act` file into the swatches.
    SwatchLoad,
    /// Save the swatches as a `.aco` or `.act` file.
    SwatchSave(TableFormat),
}

impl ToolUiEvent {
    /// A `tools.options` merge patch.
    #[must_use]
    pub const fn options(patch: Value) -> Self {
        Self::Command { id: "tools.options", params: patch }
    }
}

/// View state of the toolbox and the colour panels (nothing here is part of the document).
#[derive(Debug, Default)]
pub struct ToolUi {
    /// For each tool group (keyed by its first tool) the member shown on its button.
    pub group_tool: HashMap<String, String>,
    /// The group whose flyout is open.
    pub flyout: Option<String>,
    /// The toolbox colour chip whose swatch popup is open.
    pub chip_popup: Option<ColourTarget>,
    /// What the Colour panel edits.
    pub target: ColourTarget,
    /// The selected stop of the gradient being edited.
    pub stop: usize,
    hex: String,
    hex_for: Option<Rgba8>,
    hsb: Option<(Rgba8, [f32; 3])>,
}

/// Tools the toolbox groups under another tool's button, as `(tool, group's first tool)`.
const GROUPS: [(&str, &str); 6] = [
    ("gradient_transform", "transform"),
    ("oval", "rect"),
    ("rect_primitive", "rect"),
    ("oval_primitive", "rect"),
    ("polystar", "rect"),
    ("ink", "bucket"),
];

/// The toolbar items plus the tools that the shared toolbar has no button for yet.
#[must_use]
pub fn with_extra_tools(mut tools: Vec<ToolItem>) -> Vec<ToolItem> {
    let extras = [
        ("transform", ToolItem::new("gradient_transform", "Gradient Transform Tool", Icon::FreeTransform, Some('f'))),
        ("bucket", ToolItem::new("ink", "Ink Bottle Tool", Icon::PaintBucket, Some('s'))),
    ];
    for (after, item) in extras {
        if tools.iter().any(|t| t.id == item.id) {
            continue;
        }
        let at = tools.iter().position(|t| t.id == after).map_or(tools.len(), |i| i.saturating_add(1));
        tools.insert(at, item);
    }
    tools
}

/// The group a tool belongs to: its first tool's id.
fn group_head(id: &str) -> &str {
    GROUPS.iter().find(|(tool, _)| *tool == id).map_or(id, |(_, head)| head)
}

/// The tools in toolbox order, grouped under the button that shows one of them.
#[must_use]
pub fn groups(tools: &[ToolItem]) -> Vec<Vec<&ToolItem>> {
    let mut out: Vec<(&str, Vec<&ToolItem>)> = Vec::new();
    for tool in tools {
        let head = group_head(&tool.id);
        match out.iter_mut().find(|(h, _)| *h == head) {
            Some((_, members)) => members.push(tool),
            None => out.push((head, vec![tool])),
        }
    }
    out.into_iter().map(|(_, members)| members).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_tools_are_grouped_under_their_neighbours() {
        let tools = with_extra_tools(anim_ui::toolbar::animate_tools());
        let grouped = groups(&tools);
        let ids = |g: &[&ToolItem]| g.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert!(grouped.iter().any(|g| ids(g) == ["transform", "gradient_transform"]));
        assert!(grouped.iter().any(|g| ids(g) == ["rect", "oval"]));
        assert!(grouped.iter().any(|g| ids(g) == ["bucket", "ink"]));
        // Every tool shows up in exactly one group.
        assert_eq!(grouped.iter().map(Vec::len).sum::<usize>(), tools.len());
    }

    #[test]
    fn adding_the_extra_tools_twice_changes_nothing() {
        let once = with_extra_tools(anim_ui::toolbar::animate_tools());
        assert_eq!(with_extra_tools(once.clone()), once);
    }
}
