//! The tool interface and the tool set that dispatches events to the active tool.

use serde::{Deserialize, Serialize};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, ToolEvent};
use crate::overlay::{Cursor, Overlay};
use crate::tools::{
    BoneTool, BrushTool, EraserTool, EyedropperTool, FreeTransformTool, GradientTransformTool, HandTool, InkBottleTool, LineTool, OvalTool, PaintBucketTool,
    PenTool, PencilTool, RectangleTool, SelectionTool, SubselectionTool, TextTool, ZoomTool, bone_under, bones_overlay,
};

/// Every tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Selection,
    Subselection,
    FreeTransform,
    GradientTransform,
    Line,
    Rectangle,
    Oval,
    Pen,
    Pencil,
    Brush,
    Bone,
    PaintBucket,
    InkBottle,
    Eraser,
    Eyedropper,
    Text,
    Hand,
    Zoom,
}

impl ToolId {
    /// All tools in toolbox order.
    pub const ALL: [Self; 18] = [
        Self::Selection,
        Self::Subselection,
        Self::FreeTransform,
        Self::GradientTransform,
        Self::Pen,
        Self::Text,
        Self::Line,
        Self::Rectangle,
        Self::Oval,
        Self::Pencil,
        Self::Brush,
        Self::Bone,
        Self::PaintBucket,
        Self::InkBottle,
        Self::Eyedropper,
        Self::Eraser,
        Self::Hand,
        Self::Zoom,
    ];

    /// Id used by the shared toolbar widget (`anim_ui::toolbar::animate_tools`).
    #[must_use]
    pub const fn toolbar_id(self) -> &'static str {
        match self {
            Self::Selection => "select",
            Self::Subselection => "subselect",
            Self::FreeTransform => "transform",
            Self::GradientTransform => "gradient_transform",
            Self::Line => "line",
            Self::Rectangle => "rect",
            Self::Oval => "oval",
            Self::Pen => "pen",
            Self::Pencil => "pencil",
            Self::Brush => "brush",
            Self::Bone => "bone",
            Self::PaintBucket => "bucket",
            Self::InkBottle => "ink",
            Self::Eraser => "eraser",
            Self::Eyedropper => "eyedropper",
            Self::Text => "text",
            Self::Hand => "hand",
            Self::Zoom => "zoom",
        }
    }

    /// The tool for a toolbar id.
    #[must_use]
    pub fn from_toolbar_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.toolbar_id() == id)
    }

    /// Display name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Selection => "Selection Tool",
            Self::Subselection => "Subselection Tool",
            Self::FreeTransform => "Free Transform Tool",
            Self::GradientTransform => "Gradient Transform Tool",
            Self::Line => "Line Tool",
            Self::Rectangle => "Rectangle Tool",
            Self::Oval => "Oval Tool",
            Self::Pen => "Pen Tool",
            Self::Pencil => "Pencil Tool",
            Self::Brush => "Brush Tool",
            Self::Bone => "Bone Tool",
            Self::PaintBucket => "Paint Bucket Tool",
            Self::InkBottle => "Ink Bottle Tool",
            Self::Eraser => "Eraser Tool",
            Self::Eyedropper => "Eyedropper Tool",
            Self::Text => "Text Tool",
            Self::Hand => "Hand Tool",
            Self::Zoom => "Zoom Tool",
        }
    }

    /// One-line description of what the tool does and which modifiers it understands.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Selection => {
                "Click to select, shift-click to add, drag a marquee, drag to move (alt duplicates, shift constrains), drag an edge to bend it, double-click to enter a group, symbol or text."
            }
            Self::Subselection => "Click a shape to show its points; drag points and curve handles to reshape; arrow keys nudge; delete removes the points.",
            Self::FreeTransform => {
                "Drag handles to scale (shift keeps proportions, alt scales about the transformation point), outside a corner to rotate (shift snaps to 15 degrees), along an edge to skew, the circle to move the transformation point."
            }
            Self::GradientTransform => {
                "Click a gradient or bitmap fill to show its handles: drag the centre to move it, the square to stretch it, the circles to scale or rotate it (shift snaps to 15 degrees), the triangle to move a radial focal point."
            }
            Self::Line => "Drag to draw a line; shift snaps to 45 degrees; alt draws from the centre.",
            Self::Rectangle => "Drag to draw a rectangle with the corner radius setting; shift makes a square; alt draws from the centre.",
            Self::Oval => "Drag to draw an oval; shift makes a circle; alt draws from the centre.",
            Self::Pen => "Click for corner points, drag for smooth points; click the first point or press enter/double-click to finish; escape cancels.",
            Self::Pencil => "Drag to draw a freehand line, straightened, smoothed or left as ink depending on the pencil mode.",
            Self::Brush => "Drag to paint a filled stroke whose width follows pen pressure.",
            Self::Bone => {
                "Drag on the artwork to add a bone (shift snaps to 45 degrees); drag from a bone's tip to add a child bone; drag a bone's middle to pose it with inverse kinematics, a root bone's head to move the whole armature."
            }
            Self::PaintBucket => "Click an enclosed area or a filled region to fill it with the current fill.",
            Self::InkBottle => "Click a stroke to restyle it, or a filled region to outline it, with the current stroke.",
            Self::Eraser => "Drag to erase strokes and fills under the brush; the eraser mode limits what is removed.",
            Self::Eyedropper => "Click a fill or stroke to load its style into the toolbox and switch to the bucket or ink bottle.",
            Self::Text => "Click to place a text field and type; click a field to edit it. Right-to-left text works.",
            Self::Hand => "Drag to pan the stage.",
            Self::Zoom => "Click to zoom in (alt zooms out); drag a rectangle to zoom to it.",
        }
    }

    /// The single-key shortcut.
    #[must_use]
    pub const fn shortcut(self) -> char {
        match self {
            Self::Selection => 'v',
            Self::Subselection => 'a',
            Self::FreeTransform => 'q',
            Self::GradientTransform => 'f',
            Self::Line => 'n',
            Self::Rectangle => 'r',
            Self::Oval => 'o',
            Self::Pen => 'p',
            Self::Pencil => 'y',
            Self::Brush => 'b',
            Self::Bone => 'm',
            Self::PaintBucket => 'k',
            Self::InkBottle => 's',
            Self::Eraser => 'e',
            Self::Eyedropper => 'i',
            Self::Text => 't',
            Self::Hand => 'h',
            Self::Zoom => 'z',
        }
    }

    /// The tool whose shortcut is `c`.
    #[must_use]
    pub fn from_shortcut(c: char) -> Option<Self> {
        let c = c.to_ascii_lowercase();
        Self::ALL.into_iter().find(|t| t.shortcut() == c)
    }
}

/// What a tool returns for one event.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolOutput {
    pub commands: Vec<ToolCommand>,
    /// The cursor to show after this event.
    pub cursor: Cursor,
    /// The overlay as it stands after this event, computed against the context the event was handled in (call
    /// [`Tool::overlay`] again after applying the commands to see the new selection).
    pub overlay: Vec<Overlay>,
}

impl ToolOutput {
    /// No commands, default cursor.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn cursor(cursor: Cursor) -> Self {
        Self { commands: Vec::new(), cursor, overlay: Vec::new() }
    }

    #[must_use]
    pub const fn with(cursor: Cursor, commands: Vec<ToolCommand>) -> Self {
        Self { commands, cursor, overlay: Vec::new() }
    }
}

/// An editing tool: a state machine from events to commands.
pub trait Tool {
    fn id(&self) -> ToolId;

    /// Handles one event. The returned `overlay` is left empty; [`ToolSet`] fills it.
    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput;

    /// What to draw over the stage right now.
    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay>;

    /// Abandons any gesture in progress.
    fn cancel(&mut self);

    /// True while a gesture is in progress (the pointer is down, a pen path is open, text is being edited).
    fn is_busy(&self) -> bool;

    /// True when plain keys must reach the tool instead of triggering tool shortcuts (text editing).
    fn captures_keys(&self) -> bool {
        false
    }

    /// Called when the tool becomes active; it may start something from the current selection (the text tool
    /// starts editing a selected text field).
    fn activated(&mut self, _ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        Vec::new()
    }
}

/// All tools, the active one, and the temporary hand tool of the space bar.
#[derive(Debug, Clone)]
pub struct ToolSet {
    active: ToolId,
    /// Set while space is held to pan: the tool to return to.
    previous: Option<ToolId>,
    selection: SelectionTool,
    subselection: SubselectionTool,
    free_transform: FreeTransformTool,
    gradient_transform: GradientTransformTool,
    line: LineTool,
    rectangle: RectangleTool,
    oval: OvalTool,
    pen: PenTool,
    pencil: PencilTool,
    brush: BrushTool,
    bone: BoneTool,
    /// Set while the Selection tool has handed a press on a bone to the Bone tool for posing.
    posing_with_bone: bool,
    paint_bucket: PaintBucketTool,
    ink_bottle: InkBottleTool,
    eraser: EraserTool,
    eyedropper: EyedropperTool,
    text: TextTool,
    hand: HandTool,
    zoom: ZoomTool,
}

impl Default for ToolSet {
    fn default() -> Self {
        Self::new(ToolId::Selection)
    }
}

impl ToolSet {
    #[must_use]
    pub fn new(active: ToolId) -> Self {
        Self {
            active,
            previous: None,
            selection: SelectionTool::default(),
            subselection: SubselectionTool::default(),
            free_transform: FreeTransformTool::default(),
            gradient_transform: GradientTransformTool::default(),
            line: LineTool::default(),
            rectangle: RectangleTool::default(),
            oval: OvalTool::default(),
            pen: PenTool::default(),
            pencil: PencilTool::default(),
            brush: BrushTool::default(),
            bone: BoneTool::default(),
            posing_with_bone: false,
            paint_bucket: PaintBucketTool,
            ink_bottle: InkBottleTool,
            eraser: EraserTool::default(),
            eyedropper: EyedropperTool,
            text: TextTool::default(),
            hand: HandTool::default(),
            zoom: ZoomTool::default(),
        }
    }

    /// The tool events currently go to (the hand while space is held).
    #[must_use]
    pub const fn active(&self) -> ToolId {
        self.active
    }

    fn tool_mut(&mut self, id: ToolId) -> &mut dyn Tool {
        match id {
            ToolId::Selection => &mut self.selection,
            ToolId::Subselection => &mut self.subselection,
            ToolId::FreeTransform => &mut self.free_transform,
            ToolId::GradientTransform => &mut self.gradient_transform,
            ToolId::Line => &mut self.line,
            ToolId::Rectangle => &mut self.rectangle,
            ToolId::Oval => &mut self.oval,
            ToolId::Pen => &mut self.pen,
            ToolId::Pencil => &mut self.pencil,
            ToolId::Brush => &mut self.brush,
            ToolId::Bone => &mut self.bone,
            ToolId::PaintBucket => &mut self.paint_bucket,
            ToolId::InkBottle => &mut self.ink_bottle,
            ToolId::Eraser => &mut self.eraser,
            ToolId::Eyedropper => &mut self.eyedropper,
            ToolId::Text => &mut self.text,
            ToolId::Hand => &mut self.hand,
            ToolId::Zoom => &mut self.zoom,
        }
    }

    fn tool(&self, id: ToolId) -> &dyn Tool {
        match id {
            ToolId::Selection => &self.selection,
            ToolId::Subselection => &self.subselection,
            ToolId::FreeTransform => &self.free_transform,
            ToolId::GradientTransform => &self.gradient_transform,
            ToolId::Line => &self.line,
            ToolId::Rectangle => &self.rectangle,
            ToolId::Oval => &self.oval,
            ToolId::Pen => &self.pen,
            ToolId::Pencil => &self.pencil,
            ToolId::Brush => &self.brush,
            ToolId::Bone => &self.bone,
            ToolId::PaintBucket => &self.paint_bucket,
            ToolId::InkBottle => &self.ink_bottle,
            ToolId::Eraser => &self.eraser,
            ToolId::Eyedropper => &self.eyedropper,
            ToolId::Text => &self.text,
            ToolId::Hand => &self.hand,
            ToolId::Zoom => &self.zoom,
        }
    }

    /// Switches tool, abandoning the gesture of the old one.
    pub fn activate(&mut self, id: ToolId) {
        if id != self.active {
            let old = self.active;
            self.tool_mut(old).cancel();
        }
        self.previous = None;
        self.active = id;
    }

    /// Abandons the gesture of the active tool.
    pub fn cancel(&mut self) {
        let id = self.active;
        self.tool_mut(id).cancel();
        if std::mem::take(&mut self.posing_with_bone) {
            self.bone.cancel();
            self.bone.set_pose_only(false);
        }
    }

    /// True while the active tool is in the middle of a gesture.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.posing_with_bone || self.tool(self.active).is_busy()
    }

    /// The overlay of the active tool. The Selection tool also shows the bones of an armature layer, which it can
    /// pose.
    #[must_use]
    pub fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let mut out = self.tool(self.active).overlay(ctx);
        if self.active == ToolId::Selection {
            out.extend(bones_overlay(ctx));
        }
        out
    }

    /// The Selection tool poses an armature when a press lands on one of its bones: the Bone tool takes the gesture
    /// until the button is released.
    fn route_pose(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> Option<ToolOutput> {
        let ToolEvent::Pointer(p) = event else {
            if self.posing_with_bone && matches!(event, ToolEvent::Cancel | ToolEvent::Key(_)) {
                let mut out = self.bone.handle(event, ctx);
                if !self.bone.is_busy() {
                    self.posing_with_bone = false;
                    self.bone.set_pose_only(false);
                }
                out.overlay = self.overlay(ctx);
                return Some(out);
            }
            return None;
        };
        if !self.posing_with_bone {
            if self.active != ToolId::Selection || p.kind != crate::event::PointerKind::Down || !bone_under(ctx, p.pos) {
                return None;
            }
            self.bone.set_pose_only(true);
            self.posing_with_bone = true;
        }
        let mut out = self.bone.handle(event, ctx);
        if !self.bone.is_busy() {
            self.posing_with_bone = false;
            self.bone.set_pose_only(false);
        }
        out.overlay = self.overlay(ctx);
        Some(out)
    }

    /// Handles an event: space-bar panning and tool shortcuts first, then the active tool.
    pub fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        if let ToolEvent::Key(key) = event {
            let idle = !self.tool(self.active).is_busy() && !self.tool(self.active).captures_keys();
            if key.key == Key::Space && !key.pressed {
                if let Some(prev) = self.previous.take() {
                    self.tool_mut(ToolId::Hand).cancel();
                    self.active = prev;
                }
                return ToolOutput::none();
            }
            if key.key == Key::Space && idle && !key.modifiers.ctrl {
                if self.previous.is_none() && self.active != ToolId::Hand {
                    self.previous = Some(self.active);
                    self.active = ToolId::Hand;
                    return ToolOutput::cursor(Cursor::Hand);
                }
                return ToolOutput::none();
            }
            if key.pressed
                && idle
                && !key.modifiers.ctrl
                && !key.modifiers.alt
                && let Key::Char(c) = key.key
                && let Some(tool) = ToolId::from_shortcut(c)
            {
                self.activate(tool);
                let mut commands = vec![ToolCommand::ActivateTool { tool }];
                commands.extend(self.tool_mut(tool).activated(ctx));
                let overlay = self.overlay(ctx);
                return ToolOutput { commands, cursor: Cursor::Default, overlay };
            }
        }
        if let Some(out) = self.route_pose(event, ctx) {
            return out;
        }
        let id = self.active;
        let snapped;
        let event = if crate::snap::tool_snaps(id) && matches!(event, ToolEvent::Pointer(_)) {
            snapped = crate::snap::snap_event(event, ctx);
            &snapped
        } else {
            event
        };
        let mut out = self.tool_mut(id).handle(event, ctx);
        let switches: Vec<ToolId> = out.commands.iter().filter_map(|c| if let ToolCommand::ActivateTool { tool } = c { Some(*tool) } else { None }).collect();
        for tool in switches {
            self.activate(tool);
            out.commands.extend(self.tool_mut(tool).activated(ctx));
        }
        out.overlay = self.overlay(ctx);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_and_toolbar_ids_are_unique_and_round_trip() {
        for tool in ToolId::ALL {
            assert_eq!(ToolId::from_shortcut(tool.shortcut()), Some(tool));
            assert_eq!(ToolId::from_toolbar_id(tool.toolbar_id()), Some(tool));
        }
        assert_eq!(ToolId::from_shortcut('V'), Some(ToolId::Selection));
        assert_eq!(ToolId::from_shortcut('!'), None);
    }
}
