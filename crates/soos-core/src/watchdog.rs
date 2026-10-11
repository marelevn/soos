//! Giving up on a line fend can't interrupt. [`crate::engine`]'s guard
//! refuses the shapes known to stall fend, before fend sees them; this
//! catches the rest. A line that runs past [`LINE_LIMIT`] shows `too slow`,
//! its thread is left to finish on its own, and the document is
//! recalculated on a fresh thread that refuses that line at once.
//!
//! A line is remembered by its key (`document::LineKeys`): its text and
//! what it depends on. So it runs again once it or a line it uses changes,
//! and not while the user types elsewhere.
//!
//! At most one thread is left running at a time. A thread can't be stopped
//! from outside, and a line can't be known to be slow before it runs, so a
//! second line that runs over while one is still running holds its whole
//! recalculation until one of the two finishes -- a newer edit included,
//! and in the app every tab, since one worker recalculates them all. The
//! slow line itself, edited, is a second line. Nothing caps that wait, and
//! more slow lines make it longer: a document with two lines like
//! `twice(cm) kg` takes about 16 seconds, one with five about 35.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::error::LineError;

/// How long one line may run.
pub(crate) const LINE_LIMIT: Duration = Duration::from_secs(2);

/// How often a recalculation that ran over looks again for the thread left
/// running to finish.
const POLL: Duration = Duration::from_millis(20);

/// The lines that ran too long, and whether a thread is still finishing one.
pub(crate) struct Watchdog {
    limit: Duration,
    /// The keys of the lines that ran over.
    slow: Mutex<HashSet<u64>>,
    /// The keys of the lines taken off `slow` once (see [`Watchdog::forgive`]).
    forgiven: Mutex<HashSet<u64>>,
    /// A thread given up on is still running.
    abandoned: AtomicBool,
}

/// The app's and soos-cli's, for the life of the process.
pub(crate) static WATCHDOG: LazyLock<Arc<Watchdog>> =
    LazyLock::new(|| Arc::new(Watchdog::new(LINE_LIMIT)));

impl Watchdog {
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            limit,
            slow: Mutex::new(HashSet::new()),
            forgiven: Mutex::new(HashSet::new()),
            abandoned: AtomicBool::new(false),
        }
    }

    fn is_slow(&self, key: u64) -> bool {
        self.slow.lock().unwrap().contains(&key)
    }

    fn mark_slow(&self, key: u64) {
        self.slow.lock().unwrap().insert(key);
    }

    /// Takes `key` off the slow list, the first time only, for a line that
    /// finished soon after the limit: a busy machine more likely held it up
    /// than fend stuck on it. A line forgiven once and slow again stays slow.
    fn forgive(&self, key: u64) -> bool {
        let first = self.forgiven.lock().unwrap().insert(key);
        if first {
            self.slow.lock().unwrap().remove(&key);
        }
        first
    }
}

/// One recalculation thread, shared with the thread waiting for it.
struct Run {
    watchdog: Arc<Watchdog>,
    state: Mutex<RunState>,
    /// Given up on, read by [`given_up`] on every check fend makes. Set
    /// under `state`'s lock, so [`Run::finish`] can't miss it.
    given_up: AtomicBool,
}

#[derive(Default)]
struct RunState {
    /// The key of the line being evaluated, and when it started.
    line: Option<(u64, Instant)>,
    finished: bool,
}

impl Run {
    /// Marks the thread done; if it was given up on, another may be now.
    fn finish(&self) {
        let mut state = self.state.lock().unwrap();
        state.finished = true;
        if self.given_up.load(Ordering::Relaxed) {
            self.watchdog.abandoned.store(false, Ordering::Release);
        }
    }

    /// The thread's result, or `None` once it is given up on.
    fn wait<T>(&self, result: &Receiver<T>) -> Option<T> {
        let limit = self.watchdog.limit;
        loop {
            let left = match self.state.lock().unwrap().line {
                Some((_, since)) => (since + limit).saturating_duration_since(Instant::now()),
                None => limit,
            };
            match result.recv_timeout(left.max(POLL)) {
                Ok(out) => return Some(out),
                Err(RecvTimeoutError::Disconnected) => panic!("recalc thread panicked"),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let state = self.state.lock().unwrap();
            let Some((key, since)) = state.line else {
                continue;
            };
            if since.elapsed() < limit || state.finished {
                continue;
            }
            self.watchdog.mark_slow(key);
            let free = self
                .watchdog
                .abandoned
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            if free {
                self.given_up.store(true, Ordering::Relaxed);
                return None;
            }
        }
    }
}

thread_local! {
    /// The run this thread is, if it is one -- see [`run`].
    static RUN: RefCell<Option<Arc<Run>>> = const { RefCell::new(None) };
}

/// `recalc` on a new thread with a `stack_size` stack, watched: if a line
/// on it runs too long, `recalc` runs again on another thread, unless
/// `cancelled` says the caller no longer wants it. Then, or if `recalc`
/// returns `None` itself, this is `None`.
pub(crate) fn run<T: Send + 'static>(
    watchdog: &Arc<Watchdog>,
    stack_size: usize,
    cancelled: &dyn Fn() -> bool,
    recalc: impl Fn() -> Option<T> + Send + Sync + 'static,
) -> Option<T> {
    let recalc = Arc::new(recalc);
    loop {
        let run = Arc::new(Run {
            watchdog: Arc::clone(watchdog),
            state: Mutex::default(),
            given_up: AtomicBool::new(false),
        });
        let (done, result) = mpsc::channel();
        let (on_thread, recalc) = (Arc::clone(&run), Arc::clone(&recalc));
        std::thread::Builder::new()
            .name("soos-lines".to_owned())
            .stack_size(stack_size)
            .spawn(move || {
                RUN.set(Some(Arc::clone(&on_thread)));
                let out = recalc();
                on_thread.finish();
                let _ = done.send(out);
            })
            .expect("failed to spawn recalc thread");
        match run.wait(&result) {
            Some(out) => return out,
            None if cancelled() => return None,
            None => {}
        }
    }
}

/// `f`, the evaluation of the line or converter `key`, unless that ran too
/// long before. `None` if it did, or does now: the line is `too slow`.
/// Outside [`run`] (a test calling the document directly), just `f`.
pub(crate) fn watch<T>(key: u64, f: impl FnOnce() -> T) -> Option<T> {
    let Some(run) = RUN.with_borrow(Clone::clone) else {
        return Some(f());
    };
    if run.watchdog.is_slow(key) {
        return None;
    }
    run.state.lock().unwrap().line = Some((key, Instant::now()));
    let out = f();
    let since = run
        .state
        .lock()
        .unwrap()
        .line
        .take()
        .map(|(_, since)| since);
    let took = since.map_or(Duration::ZERO, |since| since.elapsed());
    if took < run.watchdog.limit {
        return Some(out);
    }
    // Within half as long again of the limit, it's taken at its word once.
    let soon_after = took < run.watchdog.limit * 3 / 2;
    if soon_after && run.watchdog.forgive(key) {
        return (!run.given_up.load(Ordering::Relaxed)).then_some(out);
    }
    run.watchdog.mark_slow(key);
    None
}

/// Whether this thread was given up on, so the lines after the slow one
/// stop at once (see [`crate::engine::cancelled`]).
pub(crate) fn given_up() -> bool {
    RUN.with_borrow(|run| {
        run.as_ref()
            .is_some_and(|run| run.given_up.load(Ordering::Relaxed))
    })
}

/// What a line that ran too long shows.
pub(crate) fn too_complex() -> LineError {
    LineError::own(
        "this line takes too long to calculate (over 2 seconds); it runs again when you change it or a line it uses",
        "too complex",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for fend on a line it can't be interrupted on: `sleep`
    /// can't be either.
    fn lines(keys: &'static [(u64, u64)]) -> impl Fn() -> Option<Vec<&'static str>> + Send + Sync {
        move || {
            let shown = keys
                .iter()
                .take_while(|_| !given_up())
                .map(|&(key, millis)| {
                    watch(key, || std::thread::sleep(Duration::from_millis(millis)))
                        .map_or("too slow", |()| "done")
                })
                .collect();
            (!given_up()).then_some(shown)
        }
    }

    #[test]
    fn a_line_over_the_limit_is_given_up_on_and_refused_after() {
        let watchdog = Arc::new(Watchdog::new(Duration::from_millis(100)));
        let start = Instant::now();
        let shown = run(
            &watchdog,
            1 << 20,
            &|| false,
            lines(&[(1, 0), (2, 5000), (3, 0)]),
        );
        assert_eq!(shown, Some(vec!["done", "too slow", "done"]));
        // Well before line 2's 5 s, however slow the machine.
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );

        // Run again, as for an edit to another line: refused at once.
        let start = Instant::now();
        let shown = run(&watchdog, 1 << 20, &|| false, lines(&[(2, 5000), (4, 0)]));
        assert_eq!(shown, Some(vec!["too slow", "done"]));
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
    }

    /// While one thread is still finishing a slow line, a second slow line
    /// waits for it instead of leaving a second thread running.
    #[test]
    fn only_one_thread_is_left_running() {
        let watchdog = Arc::new(Watchdog::new(Duration::from_millis(100)));
        let start = Instant::now();
        let shown = run(
            &watchdog,
            1 << 20,
            &|| false,
            lines(&[(1, 1000), (2, 8000), (3, 0)]),
        );
        assert_eq!(shown, Some(vec!["too slow", "too slow", "done"]));
        // Line 1 is given up on at 0.1 s and line 2 runs over at 0.2 s, but
        // its thread is given up on only once line 1's ends, at 1 s: well
        // before line 2's own 8 s.
        let took = start.elapsed();
        assert!(
            took >= Duration::from_millis(1000) && took < Duration::from_secs(5),
            "{took:?}"
        );
    }

    /// A line that finishes soon after the limit runs again once: a busy
    /// machine may have held it up. Slow again, it stays slow.
    #[test]
    fn a_line_just_over_the_limit_is_forgiven_once() {
        let limit = Duration::from_millis(400);
        let watchdog = Arc::new(Watchdog::new(limit));
        let just_over = || lines(&[(1, 450)]);
        let settle = || std::thread::sleep(Duration::from_millis(300));

        assert_eq!(
            run(&watchdog, 1 << 20, &|| false, just_over()),
            Some(vec!["too slow"])
        );
        settle();
        let start = Instant::now();
        assert_eq!(
            run(&watchdog, 1 << 20, &|| false, just_over()),
            Some(vec!["too slow"])
        );
        assert!(start.elapsed() >= limit, "forgiven, so it ran again");
        settle();
        let start = Instant::now();
        assert_eq!(
            run(&watchdog, 1 << 20, &|| false, just_over()),
            Some(vec!["too slow"])
        );
        assert!(
            start.elapsed() < limit,
            "refused at once: {:?}",
            start.elapsed()
        );
    }

    /// A cancelled recalculation isn't run again once its slow line is
    /// given up on.
    #[test]
    fn a_cancelled_recalculation_is_not_run_again() {
        let watchdog = Arc::new(Watchdog::new(Duration::from_millis(100)));
        let shown = run(&watchdog, 1 << 20, &|| true, lines(&[(1, 500)]));
        assert_eq!(shown, None);
    }
}
