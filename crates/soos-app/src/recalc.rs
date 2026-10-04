//! Recalculation on a worker thread. The whole document is recalculated on
//! every edit and a line may take up to fend's time limit, so doing it on
//! the UI thread would hold up typing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::*;

/// Once results have been out of date this long, they're drawn dimmed.
pub(crate) const STALE_AFTER: Duration = Duration::from_millis(150);
/// How long the UI thread waits for a new result before painting the old
/// one: most documents finish well inside a frame, and painting the old
/// results under new lines for one frame would flicker.
pub(crate) const WAIT_FOR_RESULT: Duration = Duration::from_millis(12);
/// The same, for the first results after launch.
pub(crate) const WAIT_AT_LAUNCH: Duration = Duration::from_millis(250);

struct Job {
    generation: u64,
    text: String,
    converters: Vec<soos_core::RawConverter>,
    rates: RateSource,
}

pub(crate) struct Done {
    pub(crate) generation: u64,
    pub(crate) converter_results: Vec<Result<String, String>>,
    pub(crate) results: Vec<LineResult>,
}

pub(crate) struct Recalculator {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// The newest job's generation: a job with an older one is cancelled.
    newest: Arc<AtomicU64>,
}

impl Recalculator {
    /// Starts the worker; `repaint` is woken when a result is ready.
    pub(crate) fn spawn(repaint: egui::Context) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel();
        let newest = Arc::new(AtomicU64::new(0));
        let worker_newest = Arc::clone(&newest);
        std::thread::Builder::new()
            .name("soos-recalc".to_owned())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Only the newest queued edit is worth calculating.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let newest = Arc::clone(&worker_newest);
                    let generation = job.generation;
                    let Some((converter_results, results)) = soos_core::recalc_document_unless(
                        &job.text,
                        &job.converters,
                        &job.rates,
                        move || newest.load(Ordering::Relaxed) != generation,
                    ) else {
                        continue;
                    };
                    let done = Done {
                        generation,
                        converter_results,
                        results,
                    };
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                }
            })
            .expect("failed to start the recalculation thread");
        Self { jobs, done, newest }
    }

    /// Queues `text` for recalculation, cancelling any older job.
    pub(crate) fn submit(
        &self,
        generation: u64,
        text: String,
        converters: Vec<soos_core::RawConverter>,
        rates: RateSource,
    ) {
        self.newest.store(generation, Ordering::Relaxed);
        let _ = self.jobs.send(Job {
            generation,
            text,
            converters,
            rates,
        });
    }

    /// The result for `generation`, waiting up to `wait` for it. Results
    /// for older generations are dropped.
    pub(crate) fn take(&self, generation: u64, wait: Duration) -> Option<Done> {
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let done = if remaining.is_zero() {
                self.done.try_recv().ok()?
            } else {
                self.done.recv_timeout(remaining).ok()?
            };
            if done.generation == generation {
                return Some(done);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_edit_replaces_a_slow_one() {
        let recalc = Recalculator::spawn(egui::Context::default());
        let rates = RateSource::new(std::path::PathBuf::new());
        recalc.submit(1, "9999999!\n".repeat(10), Vec::new(), rates.clone());
        recalc.submit(2, "1 + 1".to_owned(), Vec::new(), rates);
        let start = Instant::now();
        let done = recalc.take(2, Duration::from_secs(5)).expect("a result");
        assert_eq!(done.results, [LineResult::Value("2".to_owned())]);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }
}
