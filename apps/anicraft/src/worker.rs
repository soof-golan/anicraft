//! The render worker: an actor that owns a copy of the document and turns "show frame N at this size" requests
//! into stage pixels (and soundtracks) on a background thread.
//!
//! Requests are latest-wins: when several `Render` jobs are waiting, only the newest is rendered. The UI thread never
//! waits for the worker; it polls [`RenderWorker::poll`] once per frame.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anicraft_cli::audio;
use anicraft_cli::project::evaluator_for;
use anim_audio::PcmBuffer;
use anim_core::{Affine, Rgba8, num};
use anim_display::{DisplayList, Image};
use anim_render::{Background, RenderOptions};
use anim_scene::{Document, MediaStore};
use anim_timeline::{EvalOptions, Evaluator};
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};

/// What the stage shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A scene by index.
    Scene(usize),
    /// A library symbol edited on its own.
    Symbol(String),
}

/// Onion skin settings for one render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Onion {
    /// Frames before the current one.
    pub before: u32,
    /// Frames after the current one.
    pub after: u32,
}

/// One stage render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderJob {
    /// Echoed in the answer, so the UI can match it to the request.
    pub id: u64,
    pub target: Target,
    /// 0-based frame.
    pub frame: u32,
    /// Output size in pixels; the whole stage is scaled to fit it.
    pub size: [u32; 2],
    pub onion: Option<Onion>,
    /// Let movie clips advance (playback and export), instead of showing their first frame.
    pub playing: bool,
    /// Render without the stage background (transparent), for compositing.
    pub transparent: bool,
}

/// What the worker is asked.
#[derive(Debug)]
pub enum Job {
    /// A new revision of the document. `media` is only sent when it changed.
    Load {
        document: Box<Document>,
        media: Option<Box<MediaStore>>,
    },
    Render(RenderJob),
    /// Mix the soundtrack of a scene from `from_frame` to its end.
    Soundtrack {
        id: u64,
        scene: usize,
        from_frame: u32,
    },
    /// A library symbol's thumbnail.
    Thumbnail {
        name: String,
        size: u32,
    },
}

/// What the worker answers.
#[derive(Debug)]
pub enum Done {
    Frame { id: u64, image: Image, took: Duration },
    Failed { id: u64, message: String },
    Sound { id: u64, track: Option<PcmBuffer>, notes: Vec<String> },
    Thumbnail { name: String, image: Option<Image> },
}

/// Handle to the worker thread.
#[derive(Debug)]
pub struct RenderWorker {
    tx: Sender<Job>,
    rx: Receiver<Done>,
    thread: Option<JoinHandle<()>>,
}

impl RenderWorker {
    /// Starts the thread. `wake` is called after every answer so the UI repaints.
    ///
    /// # Errors
    /// When the thread cannot be started.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (tx, jobs) = unbounded();
        let (done_tx, rx) = unbounded();
        let thread = std::thread::Builder::new().name("anicraft-render".to_owned()).spawn(move || run(&jobs, &done_tx, &wake))?;
        Ok(Self { tx, rx, thread: Some(thread) })
    }

    /// Queues a job.
    pub fn send(&self, job: Job) {
        // A closed channel means the worker is gone; the UI keeps working without pixels.
        let _ = self.tx.send(job);
    }

    /// Everything that finished since the last call.
    #[must_use]
    pub fn poll(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }

    /// Waits for the next answer (tests and the headless modes).
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Option<Done> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        // Closing the sender ends the worker's loop.
        let (dead, _) = unbounded();
        self.tx = dead;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Renders one job against an evaluator. Public so the headless modes (`--smoke`) share the exact code path.
///
/// # Errors
/// A readable message when rasterising fails.
pub fn render_job(ev: &mut Evaluator<'_>, job: &RenderJob) -> Result<Image, String> {
    let (width, height) = (job.size[0].max(1), job.size[1].max(1));
    let (doc_w, doc_h) = {
        let doc = ev.document();
        (doc.width.max(1.0), doc.height.max(1.0))
    };
    ev.set_options(EvalOptions { advance_movie_clips: job.playing, ..*ev.options() });
    let view = Affine::scale_non_uniform(f64::from(width) / doc_w, f64::from(height) / doc_h);
    let background = if job.transparent { Background::Transparent } else { Background::FromDisplayList };
    let options = RenderOptions::new(width, height).with_view(view).with_background(background);
    let list = target_list(ev, &job.target, job.frame);
    let background_colour = list.background;
    let current = anim_render::render(&list, ev.images(), &options.clone().with_background(Background::Transparent)).map_err(|e| e.to_string())?;
    let onion = match (&job.target, job.onion) {
        (Target::Scene(scene), Some(o)) if o.before > 0 || o.after > 0 => ev.onion(*scene, job.frame, o.before, o.after),
        _ => Vec::new(),
    };
    let mut canvas = solid(width, height, if job.transparent { None } else { background_colour });
    for (mut list, alpha) in onion {
        anim_timeline::fade(&mut list, alpha);
        let layer = anim_render::render(&list, ev.images(), &options.clone().with_background(Background::Transparent)).map_err(|e| e.to_string())?;
        over(&mut canvas, &layer);
    }
    over(&mut canvas, &current);
    Ok(canvas)
}

fn target_list(ev: &mut Evaluator<'_>, target: &Target, frame: u32) -> DisplayList {
    match target {
        Target::Scene(scene) => ev.frame(*scene, frame),
        Target::Symbol(name) => ev.frame_of_symbol(name, frame),
    }
}

/// A premultiplied image filled with `colour` (transparent for `None`).
fn solid(width: u32, height: u32, colour: Option<Rgba8>) -> Image {
    let pixel = colour.map_or([0; 4], |c| {
        let a = u16::from(c.a);
        let mul = |v: u8| u8::try_from(u16::from(v).saturating_mul(a).saturating_add(127).checked_div(255).unwrap_or(0)).unwrap_or(255);
        [mul(c.r), mul(c.g), mul(c.b), c.a]
    });
    let count = num::u32_to_usize(width).saturating_mul(num::u32_to_usize(height));
    let mut data = Vec::with_capacity(count.saturating_mul(4));
    for _ in 0..count {
        data.extend_from_slice(&pixel);
    }
    Image { width, height, premultiplied_rgba: data }
}

/// Source-over of `src` onto `dst` (both premultiplied, same size).
fn over(dst: &mut Image, src: &Image) {
    if dst.width != src.width || dst.height != src.height {
        return;
    }
    for (d, s) in dst.premultiplied_rgba.chunks_exact_mut(4).zip(src.premultiplied_rgba.chunks_exact(4)) {
        let inverse = 255_u16.saturating_sub(s.get(3).copied().map_or(0, u16::from));
        for (dc, sc) in d.iter_mut().zip(s.iter()) {
            let blended = u16::from(*dc).saturating_mul(inverse).saturating_add(127).checked_div(255).unwrap_or(0).saturating_add(u16::from(*sc));
            *dc = u8::try_from(blended.min(255)).unwrap_or(255);
        }
    }
}

fn run(jobs: &Receiver<Job>, done: &Sender<Done>, wake: &dyn Fn()) {
    let mut media = MediaStore::new();
    let mut carry: Option<Job> = None;
    loop {
        let job = match carry.take() {
            Some(job) => job,
            None => match jobs.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };
        if let Job::Load { document, media: new_media } = job {
            if let Some(new_media) = new_media {
                media = *new_media;
            }
            if serve(jobs, done, wake, &document, &media, &mut carry).is_err() {
                return;
            }
        }
        // Render and sound jobs that arrive before any document are dropped: there is nothing to show.
    }
}

/// Answers jobs for one document revision until a new `Load` arrives (left in `carry`) or the channel closes.
fn serve(jobs: &Receiver<Job>, done: &Sender<Done>, wake: &dyn Fn(), document: &Document, media: &MediaStore, carry: &mut Option<Job>) -> Result<(), ()> {
    let mut ev = evaluator_for(document, media, false);
    loop {
        let mut job = jobs.recv().map_err(|_| ())?;
        // Latest wins: skip older renders and thumbnails of the same name.
        loop {
            match jobs.try_recv() {
                Ok(next) => match (&job, next) {
                    (Job::Render(_), next @ Job::Render(_)) => job = next,
                    (_, next @ Job::Load { .. }) => {
                        *carry = Some(next);
                        return Ok(());
                    }
                    (_, next) => {
                        // Keep order for everything else: answer the current job now, then the queued one.
                        answer(&mut ev, document, media, job, done, wake);
                        job = next;
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(()),
            }
        }
        if let Job::Load { .. } = job {
            *carry = Some(job);
            return Ok(());
        }
        answer(&mut ev, document, media, job, done, wake);
    }
}

fn answer(ev: &mut Evaluator<'_>, document: &Document, media: &MediaStore, job: Job, done: &Sender<Done>, wake: &dyn Fn()) {
    let result = catch_unwind(AssertUnwindSafe(|| match job {
        Job::Load { .. } => None,
        Job::Render(job) => {
            let started = Instant::now();
            Some(match render_job(ev, &job) {
                Ok(image) => Done::Frame { id: job.id, image, took: started.elapsed() },
                Err(message) => Done::Failed { id: job.id, message },
            })
        }
        Job::Soundtrack { id, scene, from_frame } => {
            let total = document.scenes.get(scene).map_or(0, anim_scene::Timeline::frame_count);
            let span = from_frame.min(total)..total;
            Some(match audio::soundtrack(document, media, scene, &span) {
                Ok(track) => Done::Sound { id, track: track.track, notes: track.notes },
                Err(e) => Done::Failed { id, message: e.to_string() },
            })
        }
        Job::Thumbnail { name, size } => Some(Done::Thumbnail { image: thumbnail(ev, &name, size), name }),
    }));
    match result {
        Ok(Some(answer)) => {
            let _ = done.send(answer);
            wake();
        }
        Ok(None) => {}
        Err(_) => {
            let _ = done.send(Done::Failed { id: 0, message: "the renderer panicked on this frame".to_owned() });
            wake();
        }
    }
}

/// A symbol's first frame fitted into a `size` x `size` square with a transparent background.
fn thumbnail(ev: &mut Evaluator<'_>, name: &str, size: u32) -> Option<Image> {
    let bounds = ev.symbol_bounds(name, 0)?;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let list = ev.frame_of_symbol(name, 0);
    let side = f64::from(size.max(8));
    let scale = ((side - 4.0) / bounds.width()).min((side - 4.0) / bounds.height());
    let view = Affine::translate((side / 2.0, side / 2.0)) * Affine::scale(scale) * Affine::translate((-bounds.center().x, -bounds.center().y));
    let options = RenderOptions::new(size.max(8), size.max(8)).with_view(view).with_background(Background::Transparent);
    anim_render::render(&list, ev.images(), &options).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(frame: u32) -> RenderJob {
        RenderJob { id: 7, target: Target::Scene(0), frame, size: [110, 80], onion: None, playing: false, transparent: false }
    }

    #[test]
    fn the_worker_renders_a_blank_document() {
        let worker = RenderWorker::spawn(|| {}).unwrap();
        worker.send(Job::Load { document: Box::new(Document::default()), media: Some(Box::new(MediaStore::new())) });
        worker.send(Job::Render(job(0)));
        match worker.wait(Duration::from_secs(20)) {
            Some(Done::Frame { id, image, .. }) => {
                assert_eq!(id, 7);
                assert_eq!((image.width, image.height), (110, 80));
                // The default stage background is white.
                assert!(image.premultiplied_rgba.iter().all(|b| *b == 255));
            }
            other => panic!("unexpected answer {other:?}"),
        }
    }

    #[test]
    fn latest_render_wins() {
        let worker = RenderWorker::spawn(|| {}).unwrap();
        worker.send(Job::Load { document: Box::new(Document::default()), media: None });
        for id in 1..=20 {
            worker.send(Job::Render(RenderJob { id, ..job(0) }));
        }
        let mut last = 0;
        while let Some(Done::Frame { id, .. }) = worker.wait(Duration::from_millis(1500)) {
            last = id;
        }
        assert_eq!(last, 20, "the newest request is always answered");
    }

    #[test]
    fn compositing_is_source_over() {
        let mut dst = solid(2, 1, Some(Rgba8 { r: 255, g: 255, b: 255, a: 255 }));
        let src = Image { width: 2, height: 1, premultiplied_rgba: vec![0, 0, 0, 255, 0, 0, 0, 0] };
        over(&mut dst, &src);
        assert_eq!(dst.premultiplied_rgba, vec![0, 0, 0, 255, 255, 255, 255, 255]);
    }
}
