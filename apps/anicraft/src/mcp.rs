//! The running editor as an MCP server.
//!
//! The architecture is that of `anim-mcp` (`docs/mcp.md` in anicraft-libs):
//!
//! ```text
//!   MCP client ── stdio ──► AnimMcp (rmcp) ──► Backend ──┐
//!   anim-mcp bridge ── TCP + token ──► ControlServer ────┤  channels only, no locks
//!                                                        ▼
//!                                  ControlInbox ─► SessionHost<AniDoc, GuiHooks> (owned by the UI thread)
//!                                                        │ commands, batches, queries, render, export
//!                                                        ▼
//!                                              Session<AniDoc> ── the same document the window draws
//! ```
//!
//! The UI thread owns the [`SessionHost`]. Each frame it drains the inboxes, so a tool call and a mouse click go
//! through the same commands and the same undo history, and the window repaints as soon as the call has changed the
//! document. [`GuiHooks`] wraps the headless hooks of `anicraft-cli` and adds what only a window can do: screenshots
//! of the whole editor or of one panel, and "the current frame" as the default for renders.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use anicraft_cli::mcp::{AniDoc, AniHooks, INSTRUCTIONS as HEADLESS_INSTRUCTIONS, VERSION};
use anim_mcp::{
    AnimMcp, AppHooks, Confinement, ControlConfig, ControlError, ControlInbox, ControlServer, ExportFormat, ExportRequest, HookOutcome, HookResult, HostInfo, QuerySpec,
    RenderRequest, ScreenshotRequest, SessionHost, Ticket,
};
use anim_session::{Session, SizeHint};
use serde_json::{Value, json};

/// A screenshot somebody asked for, waiting for the next frame.
#[derive(Debug)]
pub struct PendingShot {
    pub request: ScreenshotRequest,
    pub ticket: Ticket,
}

/// The hooks of the running editor.
#[derive(Debug, Default)]
pub struct GuiHooks {
    inner: AniHooks,
    /// Screenshots waiting for the window to draw a frame.
    pub shots: Vec<PendingShot>,
    /// Mirrored from the window each frame for `session_status`.
    pub playing: bool,
    /// The panels the window can crop a screenshot to.
    pub panels: Vec<String>,
}

/// What the model is told when it connects to the editor.
pub const INSTRUCTIONS: &str = "This is the AniCraft editor window; what you do here appears on the person's screen. Everything the menus do \
is a tool, and every call is one undo step the person can undo with Ctrl+Z. Start with session_status and inspect_document. Use the draw_*, select_*, \
transform_*, paint_* and modify_* tools to edit the stage exactly as the mouse tools would, selection_set_property for the Properties panel, \
timeline_* for frames, layers and tweens, and render_frame or screenshot (optionally with panel: stage, timeline, library, properties, tools) to look \
at the result. editor_goto moves the playhead. Frames are 1-based like Animate's timeline. ";

impl AppHooks<AniDoc> for GuiHooks {
    fn status(&self, session: &Session<AniDoc>) -> Value {
        let mut value = self.inner.status(session);
        let state = &session.doc().editor.state;
        if let Some(map) = value.as_object_mut() {
            map.insert("playhead".into(), json!(state.frame.saturating_add(1)));
            map.insert("tool".into(), json!(state.active_tool.toolbar_id()));
            map.insert("selected".into(), json!(state.selection.items.len()));
            map.insert("playing".into(), json!(self.playing));
            map.insert("window".into(), json!(true));
        }
        value
    }

    fn queries(&self) -> Vec<QuerySpec> {
        let mut queries = self.inner.queries();
        queries.push(QuerySpec::new("editor_state", "The playhead, active tool, current layer, selection and the timeline being edited."));
        queries
    }

    fn query(&mut self, session: &Session<AniDoc>, name: &str, params: &Value, ticket: &Ticket) -> HookResult {
        if name == "editor_state" {
            let s = &session.doc().editor.state;
            let value = json!({
                "frame": s.frame.saturating_add(1),
                "layer": s.layer,
                "tool": s.active_tool.toolbar_id(),
                "timeline": serde_json::to_value(&s.timeline).unwrap_or(Value::Null),
                "selection": serde_json::to_value(&s.selection).unwrap_or(Value::Null),
                "playing": self.playing,
                "dirty": session.is_dirty(),
                "path": session.path().map(|p| p.display().to_string()),
            });
            return Ok(HookOutcome::Ready(anim_mcp::Reply::value(value)));
        }
        self.inner.query(session, name, params, ticket)
    }

    fn outline(&self, doc: &AniDoc, depth: Option<u32>) -> Result<Value, ControlError> {
        self.inner.outline(doc, depth)
    }

    fn library(&self, doc: &AniDoc) -> Result<Value, ControlError> {
        self.inner.library(doc)
    }

    fn get_property(&self, doc: &AniDoc, path: &str) -> Result<Value, ControlError> {
        self.inner.get_property(doc, path)
    }

    fn set_property(&self, doc: &mut AniDoc, path: &str, value: Value) -> Result<Value, ControlError> {
        self.inner.set_property(doc, path, value)
    }

    fn render(&mut self, doc: &AniDoc, request: &RenderRequest, ticket: &Ticket) -> HookResult {
        // "The current frame" is the playhead of the window.
        let mut request = request.clone();
        if request.frame.is_none() {
            request.frame = Some(i64::from(doc.editor.state.frame.saturating_add(1)));
        }
        self.inner.render(doc, &request, ticket)
    }

    fn screenshot(&mut self, request: &ScreenshotRequest, ticket: &Ticket) -> HookResult {
        if let Some(panel) = &request.panel
            && !self.panels.iter().any(|p| p.eq_ignore_ascii_case(panel))
        {
            return Err(ControlError::invalid(format!("unknown panel {panel:?}; use one of {}", self.panels.join(", "))));
        }
        self.shots.push(PendingShot { request: request.clone(), ticket: ticket.clone() });
        Ok(HookOutcome::Deferred)
    }

    fn export_formats(&self) -> Vec<ExportFormat> {
        self.inner.export_formats()
    }

    fn export(&mut self, doc: &AniDoc, request: &ExportRequest, ticket: &Ticket) -> HookResult {
        self.inner.export(doc, request, ticket)
    }

    fn load(&mut self, path: &Path) -> Result<AniDoc, ControlError> {
        self.inner.load(path)
    }

    fn new_document(&mut self) -> Result<AniDoc, ControlError> {
        self.inner.new_document()
    }

    fn save(&mut self, doc: &AniDoc, file: &mut File) -> io::Result<()> {
        self.inner.save(doc, file)
    }
}

/// The session host of the editor.
pub type Host = SessionHost<AniDoc, GuiHooks>;

/// Builds the host over `doc`.
#[must_use]
pub fn host(doc: AniDoc, path: Option<PathBuf>, confinement: Confinement) -> Host {
    let mut session = Session::new(doc, crate::commands::registry());
    session.set_path(path);
    let mut hooks = GuiHooks::default();
    hooks.panels = PANELS.iter().map(|p| (*p).to_owned()).collect();
    SessionHost::new(HostInfo { name: "anicraft".to_owned(), version: VERSION.to_owned() }, session, hooks).with_confinement(confinement)
}

/// The panel names a screenshot can be cropped to.
pub const PANELS: [&str; 5] = ["tools", "stage", "timeline", "properties", "library"];

/// The transports of a running editor.
#[derive(Debug, Default)]
pub struct Links {
    server: Option<ControlServer>,
    stdio: Option<ControlInbox>,
}

/// Which transports to start.
#[derive(Debug, Clone, Default)]
pub struct LinkOptions {
    /// Serve MCP on the process's stdin and stdout (nothing else may print to stdout).
    pub stdio: bool,
    /// Listen for the `anim-mcp` bridge on this loopback port (0 picks one).
    pub control_port: Option<u16>,
    /// Where to write the control token and address.
    pub token_file: Option<PathBuf>,
}

impl Links {
    /// Starts the requested transports. `wake` repaints the window when a request arrives.
    ///
    /// # Errors
    /// When a port cannot be bound or a thread cannot be started.
    pub fn start<W: Fn() + Clone + Send + 'static>(options: &LinkOptions, wake: W) -> io::Result<Self> {
        let mut links = Self::default();
        if let Some(port) = options.control_port {
            let server = ControlServer::start(&ControlConfig { port, token_file: options.token_file.clone() }, wake.clone())?;
            eprintln!("anicraft: control server on {} (token file: {})", server.addr(), options.token_file.as_ref().map_or_else(|| "none".to_owned(), |p| p.display().to_string()));
            links.server = Some(server);
        }
        if options.stdio {
            let (inbox, backend) = ControlInbox::in_process(wake)?;
            let mcp = AnimMcp::new(backend, "anicraft", VERSION).with_instructions(format!("{INSTRUCTIONS}{HEADLESS_INSTRUCTIONS}"));
            std::thread::Builder::new().name("anicraft-mcp-stdio".to_owned()).spawn(move || {
                if let Err(e) = anim_mcp::run_stdio(mcp) {
                    eprintln!("anicraft: MCP over stdio ended: {e}");
                }
            })?;
            links.stdio = Some(inbox);
        }
        Ok(links)
    }

    /// Answers every waiting request. Call once per frame on the thread that owns the host.
    pub fn pump(&self, host: &mut Host) -> usize {
        let mut handled = 0;
        if let Some(server) = &self.server {
            handled += server.inbox().pump(host);
        }
        if let Some(inbox) = &self.stdio {
            handled += inbox.pump(host);
        }
        handled
    }

    /// The address of the control server, when running.
    #[must_use]
    pub fn control_addr(&self) -> Option<std::net::SocketAddr> {
        self.server.as_ref().map(ControlServer::addr)
    }

    /// The control token, when running.
    #[must_use]
    pub fn control_token(&self) -> Option<&str> {
        self.server.as_ref().map(ControlServer::token)
    }
}

/// A rough memory figure of the session's document, for the status line.
#[must_use]
pub fn document_bytes(doc: &AniDoc) -> usize {
    doc.size_hint()
}
