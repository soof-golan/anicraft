//! Event scripts: a JSON-friendly way to drive the tools, for tests, the command line and the MCP server.
//!
//! A script names a tool and lists events. `run_script` feeds them to an [`Editor`], applying the commands as it
//! goes, and reports what each event produced. Everything here is plain serde data, so an MCP `use_tool` call is
//! just `ToolScript` in and `ScriptReport` out.
//!
//! ```json
//! { "tool": "rectangle",
//!   "events": [ {"type":"pointer","kind":"down","pos":[10,10]},
//!               {"type":"pointer","kind":"move","pos":[90,60]},
//!               {"type":"pointer","kind":"up","pos":[90,60]} ] }
//! ```

use anim_core::Point;
use serde::{Deserialize, Serialize};

use crate::command::ToolCommand;
use crate::editor::Editor;
use crate::event::{PointerEvent, ToolEvent};
use crate::overlay::{Cursor, Overlay};
use crate::selection::Selection;
use crate::tool::ToolId;

/// A tool and the events to feed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolScript {
    /// Activated before the first event; the active tool stays when absent.
    #[serde(default)]
    pub tool: Option<ToolId>,
    pub events: Vec<ToolEvent>,
}

impl ToolScript {
    /// A press and release at one point.
    #[must_use]
    pub fn click(tool: ToolId, x: f64, y: f64) -> Self {
        Self { tool: Some(tool), events: vec![PointerEvent::down(x, y).into(), PointerEvent::up(x, y).into()] }
    }

    /// A press, a straight drag in a few steps and a release.
    #[must_use]
    pub fn drag(tool: ToolId, from: (f64, f64), to: (f64, f64)) -> Self {
        let steps = 4_u8;
        let mut events: Vec<ToolEvent> = vec![PointerEvent::down(from.0, from.1).into()];
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            events.push(PointerEvent::drag((to.0 - from.0).mul_add(t, from.0), (to.1 - from.1).mul_add(t, from.1)).into());
        }
        events.push(PointerEvent::up(to.0, to.1).into());
        Self { tool: Some(tool), events }
    }

    /// A press at the first point, moves through the rest and a release at the last.
    #[must_use]
    pub fn stroke(tool: ToolId, points: &[Point]) -> Self {
        let mut events: Vec<ToolEvent> = Vec::new();
        for (i, p) in points.iter().enumerate() {
            let event = if i == 0 {
                PointerEvent::down(p.x, p.y)
            } else if i.saturating_add(1) == points.len() {
                PointerEvent::up(p.x, p.y)
            } else {
                PointerEvent::drag(p.x, p.y)
            };
            events.push(event.into());
        }
        Self { tool: Some(tool), events }
    }
}

/// What one event produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptStep {
    pub index: usize,
    pub commands: Vec<ToolCommand>,
    pub cursor: Cursor,
}

/// The outcome of a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptReport {
    /// Steps that produced at least one command.
    pub steps: Vec<ScriptStep>,
    /// Events fed before the script ended.
    pub events_run: usize,
    /// Commands applied to the document and state.
    pub commands_applied: usize,
    /// The overlay after the last event.
    pub overlay: Vec<Overlay>,
    pub selection: Selection,
    pub active_tool: ToolId,
    /// Set when an event failed to apply; the script stopped there.
    pub error: Option<String>,
}

/// Runs a script against an editor.
pub fn run_script(editor: &mut Editor, script: &ToolScript) -> ScriptReport {
    if let Some(tool) = script.tool {
        editor.select_tool(tool);
    }
    let mut steps = Vec::new();
    let mut applied = 0_usize;
    let mut events_run = 0_usize;
    let mut error = None;
    for (index, event) in script.events.iter().enumerate() {
        match editor.send(event) {
            Ok(out) => {
                events_run = events_run.saturating_add(1);
                applied = applied.saturating_add(out.commands.len());
                if !out.commands.is_empty() {
                    steps.push(ScriptStep { index, commands: out.commands, cursor: out.cursor });
                }
            }
            Err(e) => {
                error = Some(format!("event {index}: {e}"));
                break;
            }
        }
    }
    ScriptReport {
        steps,
        events_run,
        commands_applied: applied,
        overlay: editor.overlay(),
        selection: editor.state.selection.clone(),
        active_tool: editor.tools.active(),
        error,
    }
}

/// A tool as the MCP server lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub id: ToolId,
    pub toolbar_id: String,
    pub label: String,
    pub description: String,
    pub shortcut: char,
}

/// Every tool with its description.
#[must_use]
pub fn catalog() -> Vec<ToolInfo> {
    ToolId::ALL
        .into_iter()
        .map(|id| ToolInfo {
            id,
            toolbar_id: id.toolbar_id().to_owned(),
            label: id.label().to_owned(),
            description: id.description().to_owned(),
            shortcut: id.shortcut(),
        })
        .collect()
}
