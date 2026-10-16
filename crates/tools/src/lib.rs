//! `AniCraft`'s editing tools as pure state machines.
//!
//! A tool turns [`ToolEvent`]s (pointer, key, text) into [`ToolCommand`]s plus [`Overlay`] primitives for the
//! stage view. It reads the document through a [`ToolContext`] and never mutates anything; the host applies the
//! commands (this crate ships a reference executor in [`apply`] and a headless [`Editor`] around it).
//!
//! # Layers
//!
//! * [`event`], [`settings`], [`selection`], [`context`]: what goes in. Coordinates are stage pixels, y down.
//! * [`hit`]: picking and bounds. Shapes are tested through `anim-shape`; symbols, bitmaps and text through the
//!   [`BoundsProvider`] trait, so the application can supply real text extents.
//! * [`tools`] and [`tool`]: the sixteen tools, the [`Tool`] trait and the [`ToolSet`] that routes events,
//!   handles the space-bar hand and the single-key shortcuts.
//! * [`command`] and [`overlay`]: what comes out.
//! * [`shapeops`] and [`freehand`]: geometry for merge-mode drawing, the paint bucket, ink bottle, eraser, pencil
//!   smoothing and brush outlines, built on the `VectorCraft` path booleans and planar faces.
//! * [`apply`], [`state`], [`editor`], [`script`]: executing commands, the session state, a headless editor and
//!   JSON event scripts. The MCP server exposes [`script::run_script`] and [`script::catalog`] as its `use_tool`
//!   and `list_tools` tools.
//!
//! Text is inserted in logical order, so right-to-left input needs nothing special here; the shaping and caret
//! geometry belong to `anim-text` in the host.

#![cfg_attr(
    test,
    allow(
        clippy::suboptimal_flops,
        clippy::many_single_char_names,
        clippy::float_cmp,
        clippy::arithmetic_side_effects,
        clippy::manual_assert_eq,
        reason = "test code"
    )
)]

pub mod apply;
pub mod armature;
mod autokey;
pub mod colour;
pub mod command;
pub mod context;
pub mod editor;
pub mod event;
pub mod fillfit;
pub mod freehand;
pub mod hit;
pub mod lock;
pub mod mcp;
mod mcp_armature;
mod mcp_drawing;
pub mod modify;
pub mod overlay;
mod region_select;
pub mod script;
pub mod selection;
pub mod settings;
pub mod shapeops;
pub mod snap;
pub mod state;
pub mod swatches;
pub mod tool;
pub mod tools;
pub mod widthline;

pub use apply::{Applied, ApplyError, apply};
pub use command::{CaretMotion, ToolCommand};
pub use context::{BoundsProvider, LibraryBounds, Resolved, ToolContext, resolve_ref};
pub use editor::{Editor, StepOutput};
pub use event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind, ToolEvent};
pub use hit::{Pick, PickPart, pick};
pub use mcp::{ToolHost, commands as mcp_commands};
pub use overlay::{Compass, Cursor, Marker, Node, Overlay, Role};
pub use script::{ScriptReport, ScriptStep, ToolInfo, ToolScript, catalog, run_script};
pub use selection::{ElementRef, Selection};
pub use settings::{
    BrushMode, EraserMode, GapSize, LassoMode, LassoSettings, OvalSettings, PaintBrushSettings, PaintBrushStyle, PencilMode, PolyStarSettings, PolyStarStyle,
    SnapSettings, TextSettings, TipShape, ToolSettings,
};
pub use state::{EditorState, View};
pub use tool::{Tool, ToolId, ToolOutput, ToolSet};
pub use widthline::{WidthPoint, WidthProfile};
