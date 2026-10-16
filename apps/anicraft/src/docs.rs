//! The state behind the document features of the File, Edit and Control menus.
//!
//! Document tabs, the history log, the
//! Output and Compiler Errors panels, and the dialogs. Plain data and pure functions; [`crate::app`] applies them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::mcp::docfile::{Anchor, DocType, Units};
use anicraft_cli::mcp::find::{Hit, Query};
use anim_scene::Document;
use anim_session::Session;
use anim_ui::PanelId;
use anim_ui::stage::Guide;
use anim_ui::timeline::TimelineViewState;
use serde_json::Value;

use crate::prefs::Store;
use crate::recovery::{Orphan, Recovery};

/// The History panel.
pub const HISTORY_PANEL: PanelId = PanelId::Custom(200);
/// The Compiler Errors panel.
pub const ERRORS_PANEL: PanelId = PanelId::Custom(201);

/// The tab title of a panel the dock does not name itself.
#[must_use]
pub fn panel_title(panel: PanelId) -> String {
    match panel {
        p if p == HISTORY_PANEL => "History".to_owned(),
        p if p == ERRORS_PANEL => "Compiler Errors".to_owned(),
        other => crate::toolui::panel_title(other),
    }
}

/// Panels this module adds to View > Show Panel.
pub const DOC_PANELS: [PanelId; 3] = [HISTORY_PANEL, PanelId::Output, ERRORS_PANEL];

/// The order of the built-in template categories in the New Document dialog.
pub const BUILTIN_CATEGORY_ORDER: [&str; 4] = ["Basic", "Animation", "Advertising", "Presentation"];

/// Longest Output log kept.
pub const MAX_OUTPUT: usize = 2000;

// ---- history log -----------------------------------------------------------------------------------------------

/// A command the person ran, for History's Replay and Save as Command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Step {
    pub id: String,
    pub params: Value,
}

/// What each undo step of a document was, kept in step with the session's history. Steps made by pointer gestures and
/// closures have no command (`None`) and cannot be replayed.
#[derive(Debug, Clone, Default)]
pub struct StepLog {
    done: Vec<Option<Step>>,
    undone: Vec<Option<Step>>,
}

impl StepLog {
    /// Brings the log to the history's current lengths.
    pub fn sync(&mut self, undo_len: usize, redo_len: usize) {
        while self.done.len() > undo_len {
            match self.done.pop() {
                Some(step) => self.undone.push(step),
                None => break,
            }
        }
        while self.done.len() < undo_len {
            let step = self.undone.pop().unwrap_or(None);
            self.done.push(step);
        }
        // The next step to redo is the last; what a new edit discarded is the rest.
        let excess = self.undone.len().saturating_sub(redo_len);
        self.undone.drain(..excess);
    }

    /// Records that the newest undo step was made by `step`.
    pub fn record(&mut self, step: Step) {
        if let Some(last) = self.done.last_mut() {
            *last = Some(step);
        }
    }

    /// The step at `index` (0 is the oldest).
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Step> {
        if let Some(step) = self.done.get(index) {
            return step.as_ref();
        }
        // Steps that were undone follow the applied ones, the next to redo first.
        let k = index.checked_sub(self.done.len())?;
        let at = self.undone.len().checked_sub(1)?.checked_sub(k)?;
        self.undone.get(at).and_then(Option::as_ref)
    }

    /// Forgets everything (a new document).
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }
}

// ---- tabs ------------------------------------------------------------------------------------------------------

/// The view state that belongs to one document.
#[derive(Debug, Clone, Default)]
pub struct DocView {
    pub stage: Option<anim_ui::StageView>,
    pub guides: Vec<Guide>,
    pub timeline: TimelineViewState,
    pub steps: StepLog,
}

/// A document that is open in a tab but not on screen.
pub struct Parked {
    pub id: u64,
    pub session: Session<AniDoc>,
    pub view: DocView,
}

/// The open documents. The active one lives in the window's session; the others are parked here.
#[derive(Default)]
pub struct Tabs {
    next: u64,
    /// Tab ids in the order of the strip.
    pub order: Vec<u64>,
    /// The id of the active tab.
    pub active: u64,
    pub parked: Vec<Parked>,
}

impl Tabs {
    /// A strip with one tab.
    #[must_use]
    pub fn first() -> Self {
        Self { next: 2, order: vec![1], active: 1, parked: Vec::new() }
    }

    /// A fresh tab id.
    pub fn allocate(&mut self) -> u64 {
        let id = self.next.max(2);
        self.next = id.saturating_add(1);
        id
    }

    /// The tab to the right of `id` (wrapping), or to the left with `back`.
    #[must_use]
    pub fn neighbour(&self, id: u64, back: bool) -> Option<u64> {
        let at = self.order.iter().position(|t| *t == id)?;
        let len = self.order.len();
        let to = if back { at.checked_sub(1).unwrap_or_else(|| len.saturating_sub(1)) } else { at.saturating_add(1).checked_rem(len)? };
        self.order.get(to).copied()
    }

    /// The parked tab with this id.
    #[must_use]
    pub fn parked(&self, id: u64) -> Option<&Parked> {
        self.parked.iter().find(|p| p.id == id)
    }
}

/// One tab as the strip draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub id: u64,
    pub title: String,
    pub dirty: bool,
    pub active: bool,
    pub tooltip: String,
}

/// What the strip reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabEvent {
    Select(u64),
    Close(u64),
    New,
}

/// The tab title of a document: its file name, or `Untitled-N`.
#[must_use]
pub fn tab_title(path: Option<&std::path::Path>, untitled: u64) -> String {
    path.and_then(std::path::Path::file_name).map_or_else(|| format!("Untitled-{untitled}"), |n| n.to_string_lossy().into_owned())
}

// ---- output and problems ---------------------------------------------------------------------------------------

/// How serious an Output line or a problem is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// One line of the Output panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLine {
    pub level: Level,
    pub text: String,
}

/// One entry of the Compiler Errors panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub message: String,
    /// Where: scene or symbol, layer and frame.
    pub place: String,
    pub timeline: Option<anim_scene::TimelineId>,
    pub layer: usize,
    pub frame: u32,
}

/// Checks the frame scripts of a document for unbalanced brackets, strings and comments.
///
/// Also gathers the document's own consistency warnings. These are the entries of the Compiler Errors panel until the
/// `ActionScript` compiler adds its own.
#[must_use]
pub fn problems(document: &Document) -> Vec<Problem> {
    let mut out = Vec::new();
    for warning in document.validate() {
        out.push(Problem {
            level: Level::Warning,
            message: warning.to_string(),
            place: String::new(),
            timeline: warning.location.timeline.clone(),
            layer: warning.location.layer.unwrap_or(0),
            frame: warning.location.frame.unwrap_or(0),
        });
    }
    let mut check = |timeline: anim_scene::TimelineId, label: String, scene: &anim_scene::Timeline| {
        for (li, layer) in scene.layers.iter().enumerate() {
            for frame in &layer.frames {
                if let Some(script) = &frame.actionscript
                    && let Some(error) = script_syntax_error(script)
                {
                    out.push(Problem {
                        level: Level::Error,
                        message: error,
                        place: format!("{label}, layer {:?}, frame {}", layer.name, frame.index.saturating_add(1)),
                        timeline: Some(timeline.clone()),
                        layer: li,
                        frame: frame.index,
                    });
                }
            }
        }
    };
    for (i, scene) in document.scenes.iter().enumerate() {
        check(anim_scene::TimelineId::Scene(i), format!("scene {:?}", scene.name), scene);
    }
    for item in &document.library.items {
        if let anim_scene::LibraryItem::Symbol(symbol) = item {
            check(anim_scene::TimelineId::Symbol(symbol.name.clone()), format!("symbol {:?}", symbol.name), &symbol.timeline);
        }
    }
    out
}

/// The first bracket, string or comment error of a script, if any.
#[must_use]
pub fn script_syntax_error(script: &str) -> Option<String> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut chars = script.chars().peekable();
    let mut line = 1_usize;
    while let Some(c) = chars.next() {
        match c {
            '\n' => line = line.saturating_add(1),
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        line = line.saturating_add(1);
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let start = line;
                let mut closed = false;
                let mut prev = ' ';
                for next in chars.by_ref() {
                    if next == '\n' {
                        line = line.saturating_add(1);
                    }
                    if prev == '*' && next == '/' {
                        closed = true;
                        break;
                    }
                    prev = next;
                }
                if !closed {
                    return Some(format!("1 error: unterminated comment opened on line {start}"));
                }
            }
            '"' | '\'' => {
                let start = line;
                let mut closed = false;
                while let Some(next) = chars.next() {
                    match next {
                        '\\' => {
                            chars.next();
                        }
                        '\n' => {
                            line = line.saturating_add(1);
                            break;
                        }
                        n if n == c => {
                            closed = true;
                            break;
                        }
                        _ => {}
                    }
                }
                if !closed {
                    return Some(format!("1 error: unterminated string on line {start}"));
                }
            }
            '(' | '[' | '{' => stack.push((c, line)),
            ')' | ']' | '}' => {
                let want = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match stack.pop() {
                    Some((open, _)) if open == want => {}
                    Some((open, at)) => return Some(format!("1 error: '{c}' on line {line} does not match '{open}' on line {at}")),
                    None => return Some(format!("1 error: unexpected '{c}' on line {line}")),
                }
            }
            _ => {}
        }
    }
    stack.pop().map(|(open, at)| format!("1 error: '{open}' on line {at} is never closed"))
}

// ---- dialogs ---------------------------------------------------------------------------------------------------

/// Which tab of the New Document dialog is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewTab {
    #[default]
    Presets,
    Templates,
}

/// A template the New Document dialog can start from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateChoice {
    Builtin(&'static str),
    User(PathBuf),
}

/// The New Document dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDocState {
    pub tab: NewTab,
    /// Index into [`anicraft_cli::mcp::docfile::CATEGORIES`].
    pub category: usize,
    /// Id of the chosen preset, or `None` for custom values.
    pub preset: Option<&'static str>,
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    pub kind: DocType,
    pub units: Units,
    pub background: [u8; 3],
    pub template: Option<TemplateChoice>,
    pub error: Option<String>,
}

impl Default for NewDocState {
    fn default() -> Self {
        let mut state = Self {
            tab: NewTab::Presets,
            category: 0,
            preset: None,
            width: 550.0,
            height: 400.0,
            fps: 24.0,
            kind: DocType::ActionScript3,
            units: Units::Pixels,
            background: [255, 255, 255],
            template: None,
            error: None,
        };
        state.choose(0, "hdtv_1080");
        state
    }
}

impl NewDocState {
    /// Chooses a preset of a category and copies its numbers.
    pub fn choose(&mut self, category: usize, id: &str) {
        if let Some(preset) = anicraft_cli::mcp::docfile::CATEGORIES.get(category).and_then(|c| c.presets.iter().find(|p| p.id == id)) {
            self.category = category;
            self.preset = Some(preset.id);
            self.width = preset.width;
            self.height = preset.height;
            self.fps = preset.fps;
            self.kind = preset.kind;
        }
    }

    /// The spec the dialog describes.
    #[must_use]
    pub fn spec(&self) -> anicraft_cli::mcp::docfile::NewSpec {
        let [r, g, b] = self.background;
        anicraft_cli::mcp::docfile::NewSpec {
            width: self.width,
            height: self.height,
            fps: self.fps,
            background: anim_core::Rgba8 { r, g, b, a: 255 },
            kind: self.kind,
            units: self.units,
        }
    }
}

/// Document Settings while it is being edited; applied with OK.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsState {
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    pub background: [u8; 3],
    pub units: Units,
    pub kind: DocType,
    pub scale_content: bool,
    pub anchor: Anchor,
}

impl SettingsState {
    /// The dialog's starting values: the document's own.
    #[must_use]
    pub fn of(document: &Document) -> Self {
        let bg = document.background_color;
        Self {
            width: document.width,
            height: document.height,
            fps: document.frame_rate,
            background: [bg.r, bg.g, bg.b],
            units: anicraft_cli::mcp::docfile::units(document),
            kind: anicraft_cli::mcp::docfile::doc_type(document),
            scale_content: false,
            anchor: Anchor::Center,
        }
    }

    /// The `document.settings` parameters this describes.
    #[must_use]
    pub fn params(&self) -> Value {
        let [r, g, b] = self.background;
        serde_json::json!({
            "width": self.width,
            "height": self.height,
            "frame_rate": self.fps,
            "background": format!("#{r:02x}{g:02x}{b:02x}"),
            "doc_type": self.kind.id(),
            "units": self.units.id(),
            "scale_content": self.scale_content,
            "anchor": self.anchor.id(),
        })
    }
}

/// What to do once the unsaved documents in a prompt have been dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// Close these tabs only.
    CloseTabs,
    /// Quit the application.
    Quit,
}

/// The play toggles of the Control menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayToggle {
    MuteSounds,
    SimpleButtons,
    FrameActions,
}

/// What the document menus, the tab strip and the History, Output and Compiler Errors panels ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocAction {
    /// File > New...: the New Document dialog.
    NewDialog,
    /// File > New from Template.
    NewFrom(TemplateChoice),
    SaveAll,
    Revert,
    SaveCompact,
    /// File > Save as Template...
    SaveTemplate,
    /// Close the active tab.
    CloseTab,
    CloseAll,
    SelectTab(u64),
    NextTab,
    PrevTab,
    /// File > Quit, asking about unsaved documents.
    Quit,
    /// Edit > Find and Replace...
    Find,
    FindAgain,
    /// Autosave interval in minutes (0 is off).
    SetAutosave(u32),
    Play(PlayToggle),
    ClearRecent,
    /// Show or hide a panel.
    TogglePanel(PanelId),
    DeleteTemplate(PathBuf),
    /// History panel: go to the state with this many steps applied (0 is the document as opened).
    HistoryJump(usize),
    /// History panel: run the commands of these steps (0-based, oldest first, inclusive) again.
    HistoryReplay { from: usize, to: usize },
    /// History panel: keep these steps as a named command.
    SaveSteps { from: usize, to: usize, name: String },
    /// Commands menu: run a saved command.
    RunCommand(String),
    /// Commands menu: delete a saved command.
    DeleteCommand(String),
    GotoProblem(usize),
    /// Modify > Document...
    OpenSettings,
}

/// A dialog of this module (the others are [`crate::state::Dialog`]).
#[derive(Debug, Clone, PartialEq)]
pub enum DocDialog {
    New(NewDocState),
    SaveTemplate { name: String, category: String, error: Option<String> },
    /// Save changes first? `dirty` lists the tabs still to ask about, `close` all the tabs that will be closed.
    SaveChanges { close: Vec<u64>, dirty: Vec<u64>, after: After },
    Revert,
    Recover(Vec<Orphan>),
}

/// The Find and Replace window.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: Query,
    pub hits: Vec<Hit>,
    /// The hit last shown, as an index into `hits`.
    pub current: Option<usize>,
    pub message: String,
    /// A search was run for the current query.
    pub searched: bool,
    pub color_find: [u8; 3],
    pub color_replace: [u8; 3],
    pub focus: bool,
}

/// All the UI state of this module.
#[derive(Default)]
pub struct DocsUi {
    pub dialog: Option<DocDialog>,
    pub settings: Option<SettingsState>,
    pub find: FindState,
    pub output: Vec<OutputLine>,
    pub output_min: Option<Level>,
    pub output_follow: bool,
    pub problems: Vec<Problem>,
    /// The document revision `problems` was computed for.
    pub problems_for: Option<(u64, usize, usize)>,
    pub history_selected: Option<(usize, usize)>,
    pub save_command_open: bool,
    pub save_command_name: String,
    pub commands: Vec<String>,
    pub templates: Vec<crate::templates::UserTemplate>,
    pub newdoc: Option<NewDocState>,
}

impl DocsUi {
    /// Appends a line to the Output panel.
    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        self.output.push(OutputLine { level, text: text.into() });
        if self.output.len() > MAX_OUTPUT {
            let extra = self.output.len().saturating_sub(MAX_OUTPUT);
            self.output.drain(..extra);
        }
    }
}

/// Everything about the open documents that is not drawn: tabs, settings, recovery, bookkeeping.
pub struct Docs {
    pub prefs: Store,
    pub tabs: Tabs,
    /// The history log of the active document (parked documents keep theirs in their [`DocView`]).
    pub steps: StepLog,
    pub recovery: Option<Recovery>,
    pub last_autosave: Instant,
    pub last_heartbeat: Instant,
    /// The history stamp each tab was last autosaved at.
    pub autosaved: HashMap<u64, (usize, usize, u64)>,
    /// A file being re-read to revert the active document to it.
    pub reverting: Option<PathBuf>,
    /// Counter for `Untitled-N` names.
    pub untitled: HashMap<u64, u64>,
    pub untitled_next: u64,
    /// Mute Sounds has been applied to the audio output.
    pub muted_applied: bool,
    /// The person agreed to quit: the window may close.
    pub quit_ok: bool,
}

impl Docs {
    /// The bookkeeping for a window that opens one untitled document.
    #[must_use]
    pub fn new(prefs: Store) -> Self {
        let recovery = prefs.recovery_dir().map(|dir| Recovery::new(dir, &Recovery::run_name()));
        let mut untitled = HashMap::new();
        untitled.insert(1, 1);
        Self {
            prefs,
            tabs: Tabs::first(),
            steps: StepLog::default(),
            recovery,
            last_autosave: Instant::now(),
            last_heartbeat: Instant::now(),
            autosaved: HashMap::new(),
            reverting: None,
            untitled,
            untitled_next: 2,
            muted_applied: false,
            quit_ok: false,
        }
    }

    /// Gives a tab its `Untitled-N` number.
    pub fn number_tab(&mut self, id: u64) -> u64 {
        let n = self.untitled_next;
        self.untitled_next = n.saturating_add(1);
        self.untitled.insert(id, n);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_log_follows_undo_and_redo() {
        let mut log = StepLog::default();
        let step = |n: &str| Step { id: n.to_owned(), params: Value::Null };
        log.sync(1, 0);
        log.record(step("a"));
        log.sync(2, 0);
        log.record(step("b"));
        log.sync(3, 0); // a gesture: no command
        assert_eq!(log.get(1).map(|s| s.id.as_str()), Some("b"));
        assert!(log.get(2).is_none());
        log.sync(1, 2); // undo twice
        assert_eq!(log.get(0).map(|s| s.id.as_str()), Some("a"));
        assert_eq!(log.get(1).map(|s| s.id.as_str()), Some("b"), "the next step to redo follows the applied ones");
        assert!(log.get(2).is_none(), "the gesture has no command");
        log.sync(2, 1); // redo once restores "b"
        assert_eq!(log.get(1).map(|s| s.id.as_str()), Some("b"));
        log.sync(3, 0); // a new edit after undo drops the redo stack
        assert!(log.get(2).is_none());
    }

    #[test]
    fn neighbours_wrap() {
        let mut tabs = Tabs::first();
        let two = tabs.allocate();
        tabs.order.push(two);
        assert_eq!(tabs.neighbour(1, false), Some(two));
        assert_eq!(tabs.neighbour(two, false), Some(1));
        assert_eq!(tabs.neighbour(1, true), Some(two));
    }

    #[test]
    fn scripts_are_checked_for_balance() {
        assert_eq!(script_syntax_error("function f(a:int):void { if (a > 1) { trace(\"x}\"); } } // ok }"), None);
        assert!(script_syntax_error("function f() { trace(1);").unwrap().contains("never closed"));
        assert!(script_syntax_error("var a = (1 + 2));").unwrap().contains("unexpected"));
        assert!(script_syntax_error("var s = \"open").unwrap().contains("unterminated string"));
        assert!(script_syntax_error("/* never ends").unwrap().contains("comment"));
        assert!(script_syntax_error("var a = [1, 2};").unwrap().contains("does not match"));
    }

    #[test]
    fn problems_list_script_errors_with_their_place() {
        let mut doc = Document::default();
        doc.scenes[0].layers[0].frames[0].actionscript = Some("stop(;".to_owned());
        let found = problems(&doc);
        let error = found.iter().find(|p| p.level == Level::Error).unwrap();
        assert!(error.place.contains("frame 1"), "{}", error.place);
        assert_eq!(error.layer, 0);
    }

    #[test]
    fn dialogs_start_on_a_valid_preset() {
        let state = NewDocState::default();
        assert_eq!((state.width, state.height), (1920.0, 1080.0));
        assert!(state.spec().validate().is_ok());
        let mut state = state;
        state.choose(4, "leaderboard");
        assert_eq!((state.width, state.height), (728.0, 90.0));
        assert_eq!(state.kind, DocType::Html5Canvas);
    }
}
