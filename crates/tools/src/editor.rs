//! A headless editor: document, session state and tools wired together.
//!
//! This is what the CLI, the MCP server and the tests drive. The GUI application has its own command pipeline
//! (undo through `anim-session`) but calls the same tools and the same [`apply`].

use anim_scene::Document;

use crate::apply::{ApplyError, apply};
use crate::command::ToolCommand;
use crate::context::{LibraryBounds, ToolContext};
use crate::event::ToolEvent;
use crate::overlay::{Cursor, Overlay};
use crate::state::EditorState;
use crate::tool::{ToolId, ToolSet};

/// Undo steps kept.
const MAX_UNDO: usize = 64;

/// What one event did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StepOutput {
    /// The commands the tool produced (already applied).
    pub commands: Vec<ToolCommand>,
    /// The overlay after the commands were applied.
    pub overlay: Vec<Overlay>,
    pub cursor: Cursor,
}

/// Document, state and tools.
#[derive(Debug, Clone)]
pub struct Editor {
    pub document: Document,
    pub state: EditorState,
    pub tools: ToolSet,
    /// Whether the editor keeps its own undo stack. Turn it off when something else snapshots the editor (the
    /// session framework does), so that snapshots do not carry the whole history.
    pub keep_history: bool,
    undo: Vec<Document>,
    redo: Vec<Document>,
}

impl Editor {
    #[must_use]
    pub fn new(document: Document) -> Self {
        Self { document, state: EditorState::new(), tools: ToolSet::default(), keep_history: true, undo: Vec::new(), redo: Vec::new() }
    }

    /// Makes `tool` the active tool.
    pub fn select_tool(&mut self, tool: ToolId) {
        self.tools.activate(tool);
        self.state.active_tool = tool;
    }

    /// Runs `f` with a context for the current state.
    pub fn with_context<R>(&self, f: impl FnOnce(&ToolContext<'_>) -> R) -> R {
        let bounds = LibraryBounds::new(&self.document.library);
        let ctx = ToolContext {
            document: &self.document,
            timeline: &self.state.timeline,
            layer: self.state.layer,
            frame: self.state.frame,
            selection: &self.state.selection,
            settings: &self.state.settings,
            zoom: self.state.view.zoom,
            bounds: &bounds,
        };
        f(&ctx)
    }

    /// The active tool's overlay for the current state.
    #[must_use]
    pub fn overlay(&self) -> Vec<Overlay> {
        self.with_context(|ctx| self.tools.overlay(ctx))
    }

    /// Applies commands as one undo step. When one fails, everything this call did is rolled back.
    ///
    /// # Errors
    /// The first [`ApplyError`].
    pub fn run_commands(&mut self, commands: &[ToolCommand]) -> Result<(), ApplyError> {
        let edits = commands.iter().any(ToolCommand::edits_document);
        let snapshot = edits.then(|| (self.document.clone(), self.state.clone()));
        for command in commands {
            if let Err(e) = apply(&mut self.document, &mut self.state, command) {
                if let Some((document, state)) = snapshot {
                    self.document = document;
                    self.state = state;
                }
                return Err(e);
            }
        }
        if let Some((document, _)) = snapshot
            && self.keep_history
        {
            self.undo.push(document);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        Ok(())
    }

    /// Feeds one event to the active tool and applies the commands it returns. When a command fails, the commands
    /// of this event are rolled back.
    ///
    /// # Errors
    /// The first [`ApplyError`] of the commands.
    pub fn send(&mut self, event: &ToolEvent) -> Result<StepOutput, ApplyError> {
        let bounds = LibraryBounds::new(&self.document.library);
        let out = {
            let ctx = ToolContext {
                document: &self.document,
                timeline: &self.state.timeline,
                layer: self.state.layer,
                frame: self.state.frame,
                selection: &self.state.selection,
                settings: &self.state.settings,
                zoom: self.state.view.zoom,
                bounds: &bounds,
            };
            self.tools.handle(event, &ctx)
        };
        self.run_commands(&out.commands)?;
        self.state.active_tool = self.tools.active();
        Ok(StepOutput { commands: out.commands, overlay: self.overlay(), cursor: out.cursor })
    }

    /// Undoes the last document edit.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.document, previous));
        self.state.selection.items.clear();
        true
    }

    /// Redoes the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.state.selection.items.clear();
        true
    }

    /// Number of undo steps available.
    #[must_use]
    pub const fn undo_depth(&self) -> usize {
        self.undo.len()
    }
}
