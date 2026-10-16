//! Rendering the frames of an export on every core, handed back in frame order.
//!
//! A frame of an Animate document is a pure function of its number, so the frames of an export are independent. Each
//! worker thread owns its own evaluator (decoded bitmaps, shape plans, symbol caches) and renderer (scratch buffers),
//! pulls frame numbers from a channel, and sends the finished result back over another. The calling thread puts the
//! results back in order and feeds them to a sink one by one, so the output is byte for byte what a single thread
//! produces. At most `window` frames are in flight at once, which bounds memory however slow the sink is.

use std::collections::BTreeMap;
use std::thread;

use anim_export::ExportError;
use anim_render::Renderer;
use anim_scene::{Document, MediaStore};
use anim_timeline::Evaluator;
use crossbeam_channel::unbounded;

use crate::error::{CliError, Result};
use crate::project::evaluator_for;

/// What one worker thread owns for the whole export.
#[derive(Debug)]
pub struct Worker<'a, S> {
    /// Evaluates the document at a frame.
    pub ev: Evaluator<'a>,
    /// Rasterises display lists.
    pub renderer: Renderer,
    /// Whatever else the export keeps per thread, for instance a video encoder.
    pub state: S,
}

/// How many threads work, and how many results may be in flight, for `items` units of work.
fn sizes(items: usize) -> (usize, usize) {
    let workers = thread::available_parallelism().map_or(1, usize::from).min(items).max(1);
    (workers, workers.saturating_mul(2))
}

/// Runs `render(worker, ordinal, item)` for every entry of `plan` on all cores and calls `sink(ordinal, result)` in
/// plan order.
///
/// An entry is a frame number, or whatever the caller maps to a unit of work. `init` builds the per-thread
/// [`Worker::state`] on the thread that uses it. Results should be small (encoded bytes, not pictures), since twice the
/// number of threads of them may wait for the sink. `progress(done, total)` is called before each result is handed to
/// the sink; returning false cancels with [`ExportError::Cancelled`].
///
/// # Errors
/// The first error from `init` or `render` (in plan order), from `sink`, or the cancellation.
pub fn run_ordered<S, T: Send>(
    (doc, media): (&Document, &MediaStore),
    plan: &[u32],
    init: impl Fn() -> Result<S> + Sync,
    render: impl Fn(&mut Worker<'_, S>, u32, u32) -> Result<T> + Sync,
    mut progress: impl FnMut(u32, u32) -> bool,
    mut sink: impl FnMut(u32, T) -> Result<()>,
) -> Result<()> {
    let total = plan.len();
    let total_u32 = u32::try_from(total).unwrap_or(u32::MAX);
    let (workers, window) = sizes(total);
    let (job_tx, job_rx) = unbounded::<(usize, u32)>();
    let (result_tx, result_rx) = unbounded::<(usize, Result<T>)>();
    let (init, render) = (&init, &render);
    // `move`: the job sender must drop when this closure returns, or the workers never see their channel close.
    thread::scope(move |scope| {
        for _ in 0..workers {
            let (job_rx, result_tx) = (job_rx.clone(), result_tx.clone());
            scope.spawn(move || {
                let state = match init() {
                    Ok(state) => state,
                    Err(e) => {
                        // Answer every job this thread takes with the failure, so the caller sees it in plan order.
                        let message = e.to_string();
                        while let Ok((ordinal, _)) = job_rx.recv() {
                            if result_tx.send((ordinal, Err(CliError::usage(message.clone())))).is_err() {
                                break;
                            }
                        }
                        return;
                    }
                };
                let mut worker = Worker { ev: evaluator_for(doc, media, true), renderer: Renderer::new(), state };
                while let Ok((ordinal, frame)) = job_rx.recv() {
                    let ordinal_u32 = u32::try_from(ordinal).unwrap_or(u32::MAX);
                    if result_tx.send((ordinal, render(&mut worker, ordinal_u32, frame))).is_err() {
                        break;
                    }
                }
            });
        }
        drop((job_rx, result_tx));
        let mut frames = plan.iter().copied().enumerate();
        let mut sent = 0_usize;
        let mut next = 0_usize;
        let mut waiting: BTreeMap<usize, Result<T>> = BTreeMap::new();
        while next < total {
            while sent.saturating_sub(next) < window {
                let Some(job) = frames.next() else { break };
                if job_tx.send(job).is_err() {
                    return Err(CliError::usage("internal: every render thread stopped"));
                }
                sent = sent.saturating_add(1);
            }
            if !progress(u32::try_from(next).unwrap_or(u32::MAX), total_u32) {
                return Err(ExportError::Cancelled.into());
            }
            while !waiting.contains_key(&next) {
                let Ok((ordinal, result)) = result_rx.recv() else {
                    return Err(CliError::usage("internal: every render thread stopped"));
                };
                waiting.insert(ordinal, result);
            }
            if let Some(result) = waiting.remove(&next) {
                sink(u32::try_from(next).unwrap_or(u32::MAX), result?)?;
            }
            next = next.saturating_add(1);
        }
        Ok(())
    })
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]
mod tests {
    use super::*;

    fn empty() -> (Document, MediaStore) {
        (Document::new(10.0, 10.0, 24.0), MediaStore::new())
    }

    #[test]
    fn results_reach_the_sink_in_plan_order() {
        let (doc, media) = empty();
        let plan: Vec<u32> = (0..500).rev().collect();
        let mut seen = Vec::new();
        run_ordered(
            (&doc, &media),
            &plan,
            || Ok(()),
            |_, ordinal, frame| {
                // Early frames finish last, so completion order differs from plan order.
                std::thread::sleep(std::time::Duration::from_micros(u64::from(500_u32.saturating_sub(ordinal)) % 7 * 100));
                Ok((ordinal, frame))
            },
            |_, _| true,
            |ordinal, (worker_ordinal, frame)| {
                assert_eq!(ordinal, worker_ordinal);
                seen.push(frame);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, plan);
    }

    #[test]
    fn cancelling_and_failing_stop_the_export() {
        let (doc, media) = empty();
        let plan: Vec<u32> = (0..100).collect();
        let cancelled = run_ordered((&doc, &media), &plan, || Ok(()), |_, _, frame| Ok(frame), |done, _| done < 10, |_, _| Ok(()));
        assert!(matches!(cancelled, Err(CliError::Export(ExportError::Cancelled))), "{cancelled:?}");
        let failed = run_ordered(
            (&doc, &media),
            &plan,
            || Ok(()),
            |_, _, frame| if frame == 40 { Err(CliError::usage("boom")) } else { Ok(frame) },
            |_, _| true,
            |_, _| Ok(()),
        );
        assert!(matches!(failed, Err(CliError::Usage(_))), "{failed:?}");
    }
}
