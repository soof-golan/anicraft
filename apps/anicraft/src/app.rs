//! The editor window: an [`eframe::App`] whose state is a `SessionHost` (document, history, MCP) plus view state.
//!
//! Every frame runs the same steps, none of which keeps UI state outside [`AniApp`]:
//!
//! 1. answer MCP requests, collect finished renders, advance playback;
//! 2. build the view models from the document ([`crate::view`], [`crate::props`]) and draw the panels, which return
//!    events instead of changing anything;
//! 3. turn the events into commands on the session ([`crate::actions`], [`crate::input`]);
//! 4. tell the render worker what to draw next, and finish screenshots.

mod actions;
mod draw;
mod fileops;
mod input;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::Project;
use anim_core::num;
use anim_mcp::{Confinement, Reply};
use anim_session::JobSet;
use anim_ui::Theme;
use egui::{ColorImage, Context, TextureHandle, TextureOptions, UserData, ViewportCommand};
use serde_json::json;

use crate::files::Loaded;
use crate::mcp::{Host, LinkOptions, Links, PendingShot};
use crate::state::UiState;
use crate::worker::{Done, Job, Onion, RenderJob, RenderWorker, Target};

/// What to do with the window before closing it, for `--screenshot`.
#[derive(Debug, Clone)]
pub struct ShotPlan {
    /// Where the PNG goes.
    pub out: PathBuf,
    /// 1-based frame to show, or the document's first.
    pub frame: Option<u32>,
    /// Crop to one panel (`stage`, `timeline`, ...).
    pub panel: Option<String>,
    /// Tool to activate before the shot, for demonstrating overlays.
    pub tool: Option<String>,
    /// Open the Properties panel on this selection: `layer:keyframe:index`.
    pub select: Option<String>,
}

/// Everything the command line can ask for.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub file: Option<PathBuf>,
    pub shot: Option<ShotPlan>,
    pub dark: bool,
    pub links: LinkOptions,
    pub roots: Vec<PathBuf>,
    pub window: Option<(f32, f32)>,
}

/// The part of a screenshot a panel occupies (the whole image without a panel).
fn crop(image: &ColorImage, rect: Option<&egui::Rect>, ppp: f32) -> ColorImage {
    rect.map_or_else(|| image.clone(), |rect| image.region(rect, Some(ppp)))
}

/// What the render worker was last asked for; two requests with equal keys draw the same pixels.
#[derive(Debug, Clone, PartialEq)]
struct RenderKey {
    revision: u64,
    target: Target,
    frame: u32,
    size: [u32; 2],
    onion: Option<(u32, u32)>,
    playing: bool,
}

#[derive(Debug, Default)]
struct Sync {
    fingerprint: Option<Fingerprint>,
    media: (usize, u64),
    revision: u64,
    next_id: u64,
    requested: Option<RenderKey>,
    inflight: HashMap<u64, RenderKey>,
    shown_id: u64,
    shown: Option<RenderKey>,
    last_took: Duration,
    last_error: Option<String>,
}

/// A cheap summary of "the document may have changed".
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    undo: usize,
    redo: usize,
    dirty: bool,
    edits: u64,
    path: Option<PathBuf>,
    scenes: usize,
}

#[derive(Debug)]
enum ShotStage {
    /// Wait for fonts, layout and the first render to settle.
    Settling { frames: u32 },
    Requested,
}

#[derive(Debug)]
struct ShotRun {
    plan: ShotPlan,
    stage: ShotStage,
}

/// The editor.
pub struct AniApp {
    pub(crate) host: Host,
    pub(crate) ui: UiState,
    pub(crate) worker: RenderWorker,
    pub(crate) jobs: JobSet<Loaded>,
    links: Links,
    stage_tex: Option<TextureHandle>,
    sync: Sync,
    /// Counts document edits made by the window itself (pointer gestures change the document outside history).
    pub(crate) edits: u64,
    shot: Option<ShotRun>,
    mcp_shots: HashMap<u64, Vec<PendingShot>>,
    next_shot: u64,
    title: String,
    pub(crate) audio: crate::audio::Audio,
    pub(crate) soundtrack_id: u64,
    fonts: bool,
    frames_drawn: u32,
    closing: bool,
    outcome: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    ctx: Context,
    /// Distinguishes property drags, so each drag is one undo step.
    prop_gesture: u64,
}

impl AniApp {
    /// Builds the window state. Loading the file happens here, before the first frame, so a bad file is reported at
    /// start-up.
    ///
    /// # Errors
    /// When the document cannot be read or a transport cannot start.
    pub fn new(ctx: &Context, options: &Options, outcome: Option<std::sync::mpsc::Sender<Result<(), String>>>) -> color_eyre::Result<Self> {
        let (doc, path) = match &options.file {
            Some(path) => (AniDoc::from(Project::open(path)?), Some(path.clone())),
            None => (AniDoc::default(), None),
        };
        let size = (doc.document().width, doc.document().height);
        let mut roots = options.roots.clone();
        if let Some(parent) = path.as_ref().and_then(|p| p.parent()).filter(|p| !p.as_os_str().is_empty()) {
            roots.push(parent.to_path_buf());
        }
        roots.retain(|r| r.is_dir());
        roots.sort();
        roots.dedup();
        let confinement = Confinement::new(&roots, &roots).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        let host = crate::mcp::host(doc, path, confinement);
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        let worker = RenderWorker::spawn(wake.clone())?;
        let links = Links::start(&options.links, wake)?;
        let rtl_fonts = anim_ui::theme::install_fallback_fonts(ctx);
        let theme = if options.dark { Theme::Dark } else { Theme::Light };
        theme.apply(ctx);
        let mut ui = UiState::new(size, theme);
        if rtl_fonts == 0 {
            "no installed font covers Hebrew or Arabic; right-to-left text will show boxes".clone_into(&mut ui.status);
        }
        let mut app = Self {
            host,
            ui,
            worker,
            jobs: JobSet::new(),
            links,
            stage_tex: None,
            sync: Sync::default(),
            edits: 0,
            shot: options.shot.clone().map(|plan| ShotRun { plan, stage: ShotStage::Settling { frames: 0 } }),
            mcp_shots: HashMap::new(),
            next_shot: 1,
            title: String::new(),
            audio: crate::audio::Audio::default(),
            soundtrack_id: 0,
            fonts: false,
            frames_drawn: 0,
            closing: false,
            outcome,
            ctx: ctx.clone(),
            prop_gesture: 0,
        };
        if let Some(run) = &app.shot {
            let plan = run.plan.clone();
            app.prepare_shot(&plan);
        }
        Ok(app)
    }

    fn prepare_shot(&mut self, plan: &ShotPlan) {
        if let Some(frame) = plan.frame {
            self.goto_frame(frame.saturating_sub(1));
        }
        if let Some(tool) = plan.tool.as_deref().and_then(anicraft_tools::ToolId::from_toolbar_id) {
            let doc = self.host.session_mut().doc_mut();
            doc.editor.select_tool(tool);
        }
        if let Some(spec) = &plan.select {
            let parts: Vec<usize> = spec.split(':').filter_map(|p| p.parse().ok()).collect();
            if let [layer, keyframe, index] = parts.as_slice() {
                let doc = self.host.session_mut().doc_mut();
                doc.editor.state.layer = *layer;
                doc.editor.state.selection = anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(*layer, u32::try_from(*keyframe).unwrap_or(0), *index));
            }
        }
    }

    /// Moves the playhead (0-based) without recording history.
    pub(crate) fn goto_frame(&mut self, frame: u32) {
        let doc = self.host.session_mut().doc_mut();
        let last = doc.editor.document.timeline(&doc.editor.state.timeline).map_or(0, |t| t.frame_count().saturating_sub(1));
        doc.editor.state.frame = frame.min(last.max(frame.min(10_000)));
    }

    fn fingerprint(&self) -> Fingerprint {
        let session = self.host.session();
        Fingerprint {
            undo: session.history().undo_len(),
            redo: session.history().redo_len(),
            dirty: session.is_dirty(),
            edits: self.edits,
            path: session.path().map(std::path::Path::to_path_buf),
            scenes: session.doc().document().scenes.len(),
        }
    }

    /// Step 1 of the frame: everything that arrives from outside the window.
    fn receive(&mut self, ctx: &Context) {
        let handled = self.links.pump(&mut self.host);
        if handled > 0 {
            ctx.request_repaint();
        }
        self.host.hooks_mut().playing = self.ui.playback.is_some();
        self.collect_shots(ctx);
        self.poll_worker(ctx);
        self.poll_jobs();
        self.tick_playback(ctx);
        let title = self.host.session().title("AniCraft");
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn poll_worker(&mut self, ctx: &Context) {
        for done in self.worker.poll() {
            match done {
                Done::Frame { id, image, took } => {
                    if id < self.sync.shown_id {
                        continue;
                    }
                    self.sync.shown_id = id;
                    self.sync.last_took = took;
                    self.sync.last_error = None;
                    self.sync.shown = self.sync.inflight.remove(&id);
                    self.sync.inflight.retain(|k, _| *k > id);
                    let size = [num::u32_to_usize(image.width), num::u32_to_usize(image.height)];
                    let colour = ColorImage::from_rgba_premultiplied(size, &image.premultiplied_rgba);
                    match &mut self.stage_tex {
                        Some(tex) => tex.set(colour, TextureOptions::LINEAR),
                        None => self.stage_tex = Some(ctx.load_texture("stage", colour, TextureOptions::LINEAR)),
                    }
                }
                Done::Failed { id: _, message } => {
                    self.sync.last_error = Some(message.clone());
                    self.ui.status = format!("render failed: {message}");
                }
                Done::Sound { id, track, notes } => self.soundtrack_ready(id, track, &notes),
                Done::Thumbnail { name, image } => {
                    if let Some(image) = image {
                        let size = [num::u32_to_usize(image.width), num::u32_to_usize(image.height)];
                        let colour = ColorImage::from_rgba_premultiplied(size, &image.premultiplied_rgba);
                        let tex = ctx.load_texture(format!("thumb:{name}"), colour, TextureOptions::LINEAR);
                        self.ui.thumbs.insert(name, tex);
                    }
                }
            }
        }
    }

    /// Step 4: keep the worker's document in step with the session and ask for the pixels the stage needs.
    fn sync_worker(&mut self, ctx: &Context) {
        let fingerprint = self.fingerprint();
        let doc = self.host.session().doc();
        let media_now = (doc.media.len(), doc.media.total_bytes());
        if self.sync.fingerprint.as_ref() != Some(&fingerprint) || self.sync.revision == 0 {
            self.sync.revision = self.sync.revision.saturating_add(1);
            let media = (media_now != self.sync.media || self.sync.revision == 1).then(|| Box::new(doc.media.clone()));
            self.sync.media = media_now;
            self.worker.send(Job::Load { document: Box::new(doc.document().clone()), media });
            self.sync.fingerprint = Some(fingerprint);
        }
        let state = &doc.editor.state;
        let scenes = doc.document().scenes.len().max(1);
        let target = match &state.timeline {
            anim_scene::TimelineId::Scene(i) => Target::Scene((*i).min(scenes.saturating_sub(1))),
            anim_scene::TimelineId::Symbol(name) => Target::Symbol(name.clone()),
        };
        let size = self.ui.stage.desired_texture_size(ctx.pixels_per_point());
        let onion = (self.ui.onion.enabled && self.ui.playback.is_none()).then_some((self.ui.onion.before, self.ui.onion.after));
        let key = RenderKey { revision: self.sync.revision, target: target.clone(), frame: state.frame, size, onion, playing: self.ui.playback.is_some() };
        if self.sync.requested.as_ref() != Some(&key) {
            self.sync.next_id = self.sync.next_id.saturating_add(1);
            let id = self.sync.next_id;
            self.worker.send(Job::Render(RenderJob {
                id,
                target,
                frame: key.frame,
                size,
                onion: onion.map(|(before, after)| Onion { before, after }),
                playing: key.playing,
                transparent: false,
            }));
            self.sync.inflight.insert(id, key.clone());
            self.sync.requested = Some(key);
        }
        self.request_thumbnails();
    }

    fn request_thumbnails(&mut self) {
        let doc = self.host.session().doc().document();
        let mut budget = 6;
        for symbol in doc.library.symbols() {
            if budget == 0 {
                break;
            }
            if self.ui.thumbs.contains_key(&symbol.name) || self.ui.thumb_requested.contains(&symbol.name) {
                continue;
            }
            if symbol.name.rsplit_once('/').is_some_and(|(parent, _)| self.ui.collapsed_folders.contains(parent)) {
                continue;
            }
            self.ui.thumb_requested.insert(symbol.name.clone());
            self.worker.send(Job::Thumbnail { name: symbol.name.clone(), size: 48 });
            budget = usize::saturating_sub(budget, 1);
        }
    }

    /// Forgets everything derived from the previous document (after Open or New).
    pub(crate) fn reset_view_for_new_document(&mut self) {
        let doc = self.host.session().doc().document();
        self.ui.stage.doc_size = egui::vec2(num::f64_to_f32(doc.width), num::f64_to_f32(doc.height));
        self.ui.stage.fit_request = true;
        self.ui.thumbs.clear();
        self.ui.thumb_requested.clear();
        self.ui.collapsed_folders.clear();
        self.ui.collapsed_layers.clear();
        self.ui.library_selected = None;
        self.ui.timeline.selection = anim_ui::timeline::FrameSelection::default();
        self.ui.timeline.selected_layers.clear();
        self.stop_playback();
        self.sync.requested = None;
        self.sync.fingerprint = None;
        self.sync.revision = 0;
    }

    fn collect_shots(&mut self, ctx: &Context) {
        let pending: Vec<PendingShot> = std::mem::take(&mut self.host.hooks_mut().shots);
        if !pending.is_empty() {
            let tag = self.next_shot;
            self.next_shot = self.next_shot.saturating_add(1);
            self.mcp_shots.insert(tag, pending);
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(tag)));
            ctx.request_repaint();
        }
        let events: Vec<(UserData, ColorImage)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => Some((user_data.clone(), (**image).clone())),
                    _ => None,
                })
                .collect()
        });
        for (data, image) in events {
            let tag = data.data.as_ref().and_then(|d| d.downcast_ref::<u64>().copied());
            match tag {
                Some(0) | None => self.finish_cli_shot(&image),
                Some(tag) => self.finish_mcp_shots(tag, &image, ctx.pixels_per_point()),
            }
        }
    }

    fn finish_mcp_shots(&mut self, tag: u64, image: &ColorImage, ppp: f32) {
        let Some(shots) = self.mcp_shots.remove(&tag) else { return };
        for shot in shots {
            let rect = shot.request.panel.as_deref().and_then(|p| self.ui.panel_rects.get(&p.to_lowercase()));
            let shown = crop(image, rect, ppp);
            let [w, h] = shown.size;
            let rgba: Vec<u8> = shown.pixels.iter().flat_map(egui::Color32::to_array).collect();
            let result = u32::try_from(w)
                .ok()
                .zip(u32::try_from(h).ok())
                .ok_or_else(|| anim_mcp::ControlError::failed("the screenshot is too large"))
                .and_then(|(w, h)| Reply::value(json!({ "width": w, "height": h, "panel": shot.request.panel })).with_png_rgba(w, h, &rgba));
            let result = result.and_then(|reply| {
                if let (Some(path), Some(img)) = (&shot.request.save_to, reply.images.first()) {
                    anim_mcp::save_image(path, img)?;
                }
                Ok(reply)
            });
            shot.ticket.finish(result);
        }
    }

    fn finish_cli_shot(&mut self, image: &ColorImage) {
        let Some(run) = self.shot.take() else { return };
        let rect = run.plan.panel.as_deref().and_then(|p| self.ui.panel_rects.get(&p.to_lowercase()));
        let shown = crop(image, rect, self.ctx.pixels_per_point());
        let [w, h] = shown.size;
        let rgba: Vec<u8> = shown.pixels.iter().flat_map(egui::Color32::to_array).collect();
        let written = u32::try_from(w)
            .ok()
            .zip(u32::try_from(h).ok())
            .ok_or_else(|| "screenshot too large".to_owned())
            .and_then(|(w, h)| anim_export::encode_png(w, h, &rgba).map_err(|e| e.to_string()))
            .and_then(|png| std::fs::write(&run.plan.out, png).map_err(|e| format!("{}: {e}", run.plan.out.display())));
        match &written {
            Ok(()) => eprintln!("anicraft: wrote {} ({w}x{h})", run.plan.out.display()),
            Err(e) => eprintln!("anicraft: screenshot failed: {e}"),
        }
        if let Some(outcome) = &self.outcome {
            let _ = outcome.send(written);
        }
        self.closing = true;
    }

    /// The `--screenshot` state machine: wait until the stage shows the requested frame, then ask for the capture.
    fn drive_shot(&mut self, ctx: &Context) {
        let ready = self.sync.shown.is_some() && self.sync.shown == self.sync.requested && self.sync.inflight.is_empty() && self.fonts && self.sync.last_error.is_none();
        let Some(run) = &mut self.shot else { return };
        match &mut run.stage {
            ShotStage::Settling { frames } => {
                if ready {
                    *frames = frames.saturating_add(1);
                } else {
                    *frames = 0;
                }
                // A few settled frames so layout, fonts, thumbnails and the dock have all been drawn.
                if *frames >= 8 {
                    run.stage = ShotStage::Requested;
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(0_u64)));
                }
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            ShotStage::Requested => ctx.request_repaint_after(Duration::from_millis(30)),
        }
        if self.frames_drawn > 1200 && !self.closing {
            eprintln!("anicraft: screenshot timed out waiting for the stage to render");
            if let Some(outcome) = &self.outcome {
                let _ = outcome.send(Err("timed out".to_owned()));
            }
            self.closing = true;
        }
    }
}
