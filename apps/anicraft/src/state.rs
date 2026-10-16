//! Everything the window remembers that is not the document: layout, view settings, dialogs and playback.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use anim_ui::dock::{self, docking::Layout};
use anim_ui::timeline::{OnionSkin, TimelineViewState};
use anim_ui::{PanelId, StageViewState, Theme, ToolItem, toolbar};
use egui::{Rect, TextureHandle, vec2};

/// A modal question to the person at the keyboard.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    /// Convert the selection to a symbol (name and kind).
    ConvertToSymbol { name: String, kind: usize },
    /// Create an empty symbol.
    NewSymbol { name: String, kind: usize },
    /// Rename the layer at this index.
    RenameLayer { index: usize, name: String },
    /// Rename a library item.
    RenameItem { old: String, name: String },
    /// Document settings (size, frame rate, background).
    DocumentSettings,
    /// Something went wrong that the person should read.
    Message { title: String, text: String },
    /// Keyboard shortcuts and about.
    About,
    /// The document has unsaved changes; run this action if the person agrees to discard them.
    Discard(Box<crate::menus::MenuAction>),
}

/// How a play session runs; the frame is derived from the clock, never counted.
#[derive(Debug, Clone, Copy)]
pub struct Playback {
    pub started: Instant,
    /// The 0-based frame playback started from.
    pub from_frame: u32,
    /// Increments each time playback (re)starts, so a late soundtrack for an old start is ignored.
    pub token: u64,
}

/// An export running on a worker.
#[derive(Debug, Clone)]
pub struct ExportStatus {
    pub label: String,
    pub fraction: f32,
}

/// View state and transient UI state.
pub struct UiState {
    pub theme: Theme,
    pub layout: Layout<PanelId>,
    pub stage: StageViewState,
    pub timeline: TimelineViewState,
    pub onion: OnionSkin,
    pub collapsed_layers: BTreeSet<u64>,
    pub collapsed_folders: BTreeSet<String>,
    pub library_selected: Option<String>,
    pub library_filter: String,
    /// The Library panel's preview of the selected item.
    pub preview: crate::preview::PreviewState,
    pub thumbs: HashMap<String, TextureHandle>,
    pub thumb_requested: BTreeSet<String>,
    pub tools: Vec<ToolItem>,
    pub status: String,
    pub dialog: Option<Dialog>,
    pub looping: bool,
    pub playback: Option<Playback>,
    pub next_token: u64,
    pub clipboard: Vec<anim_scene::Element>,
    pub frame_clipboard: Vec<(u64, u32, anim_scene::Frame)>,
    pub panel_rects: HashMap<String, Rect>,
    pub export: Option<ExportStatus>,
    /// Pointer drag in progress: the document as it was before the first edit of the gesture.
    pub drag_base: Option<Box<anicraft_cli::mcp::AniDoc>>,
    /// Whether the tool edited the document during this gesture.
    pub drag_label: String,
    pub recent: Vec<PathBuf>,
    pub show_output: bool,
    pub tool_cursor: Option<egui::CursorIcon>,
    pub tool_overlay: Vec<anim_ui::stage::Overlay>,
}

impl UiState {
    /// Fresh state for a document of the given stage size.
    #[must_use]
    pub fn new(doc_size: (f64, f64), theme: Theme) -> Self {
        let size = vec2(anim_core::num::f64_to_f32(doc_size.0), anim_core::num::f64_to_f32(doc_size.1));
        Self {
            theme,
            layout: dock::default_layout(),
            stage: StageViewState::new(size),
            timeline: TimelineViewState::default(),
            onion: OnionSkin::default(),
            collapsed_layers: BTreeSet::new(),
            collapsed_folders: BTreeSet::new(),
            library_selected: None,
            library_filter: String::new(),
            preview: crate::preview::PreviewState::default(),
            thumbs: HashMap::new(),
            thumb_requested: BTreeSet::new(),
            tools: toolbar::animate_tools(),
            status: "Ready".to_owned(),
            dialog: None,
            looping: true,
            playback: None,
            next_token: 1,
            clipboard: Vec::new(),
            frame_clipboard: Vec::new(),
            panel_rects: HashMap::new(),
            export: None,
            drag_base: None,
            drag_label: String::new(),
            recent: Vec::new(),
            show_output: false,
            tool_cursor: None,
            tool_overlay: Vec::new(),
        }
    }
}
