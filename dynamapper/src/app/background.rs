//! The app's background work: one pool of a few threads, sized from the machine, that runs every job the frame does
//! not wait on (package decodes, statics chunk builds and loads, scans), by priority. Bevy's own pools keep their own
//! work: the frame's systems (compute), asset loading (IO) and Bevy's pipeline compiles (async compute), so a burst
//! of ours never holds a shader compile back, and the reverse.
//!
//! - **Priorities**: a worker takes the most urgent job; a job that has waited [`PROMOTE_AFTER`] counts one level more
//!   urgent per wait, so idle work still runs during a long walk. A running job is never interrupted: split long work.
//! - **Lanes** ([`Lane`]): an ordered queue that keeps its state between jobs (a builder's caches). At most one of its
//!   jobs is in the pool at a time, at the level of its most urgent queued job; between jobs other work gets through.
//! - **Dropped work**: a job whose [`Task`] is dropped before it starts never runs.
//! - **Parallel parts** ([`BackgroundPool::for_each_index`], udd's decodes): the calling thread works through the parts
//!   with the workers that join it, and waits only for parts already started, so it cannot deadlock, even on one
//!   worker or from inside a job.
//! - A panicking job is logged and its worker goes on. Every job runs in a `background` span named after it, for the
//!   span trace.

use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How urgent a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// The current view waits for it: pages and chunks in view, the parts of a decode such a job started.
    Visible = 0,
    /// Wanted soon: loading ahead of the walk, chunks at the view's edge, warm-ups.
    Ahead = 1,
    /// Whenever there is time: scans, probes.
    Idle = 2,
}

const PRIORITY_LEVELS: usize = 3;
const PRIORITIES: [Priority; PRIORITY_LEVELS] = [Priority::Visible, Priority::Ahead, Priority::Idle];

/// A queued job counts one level more urgent for every `PROMOTE_AFTER` it has waited.
const PROMOTE_AFTER: Duration = Duration::from_millis(250);
/// The most worker threads: past it, a machine's spare cores go to the frame and the driver.
const WORKERS_MAX: usize = 4;
/// Hardware threads left to the rest of the app: the main and render threads and Bevy's compute pool (at most 3).
const THREADS_RESERVED: usize = 5;
/// A Visible job that waited longer than this is logged: the pool is too small for what the view asks.
const VISIBLE_WAIT_WARN: Duration = Duration::from_millis(100);

/// The pool's size on this machine: what the frame leaves, at least one thread and at most [`WORKERS_MAX`].
pub fn default_worker_count() -> usize {
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
    threads.saturating_sub(THREADS_RESERVED).clamp(1, WORKERS_MAX)
}

/// The app's pool, started on first use with [`default_worker_count`] workers.
pub fn background_pool() -> &'static BackgroundPool {
    static POOL: OnceLock<BackgroundPool> = OnceLock::new();
    POOL.get_or_init(|| BackgroundPool::new(default_worker_count()))
}

/// Hands udd's parallel decodes to the app's pool. Call once at start-up, before any package decodes.
pub fn install_udd_executor() {
    if udd_container::executor::set_parallel_executor(Box::new(UddExecutor)).is_err() {
        log::warn!("The udd decodes had started on their own executor before the background pool was installed");
    }
}

struct UddExecutor;

impl udd_container::executor::ParallelExecutor for UddExecutor {
    fn for_each_index(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        background_pool().for_each_index(count, job);
    }
}

type JobFn = Box<dyn FnOnce() + Send>;

struct Job {
    name: &'static str,
    priority: Priority,
    queued_at: Instant,
    /// Set when the job's `Task` is dropped before it starts.
    cancelled: Option<Arc<AtomicBool>>,
    run: JobFn,
}

#[derive(Default)]
struct Queues {
    levels: [VecDeque<Job>; PRIORITY_LEVELS],
    shutdown: bool,
}

impl Queues {
    /// The most urgent queued job, counting each job's wait (`PROMOTE_AFTER`); the more urgent level wins a tie.
    fn pop_most_urgent(&mut self, now: Instant) -> Option<Job> {
        let level = (0..PRIORITY_LEVELS)
            .filter_map(|level| {
                let job = self.levels[level].front()?;
                let promotions = now.saturating_duration_since(job.queued_at).as_millis() / PROMOTE_AFTER.as_millis();
                Some((level as i64 - promotions.min(PRIORITY_LEVELS as u128) as i64, level))
            })
            .min()?
            .1;
        self.levels[level].pop_front()
    }
}

struct Shared {
    queues: Mutex<Queues>,
    work: Condvar,
}

thread_local! {
    /// The priority of the job this thread runs, for the parallel parts it starts.
    static RUNNING_PRIORITY: std::cell::Cell<Option<Priority>> = const { std::cell::Cell::new(None) };
}

pub struct BackgroundPool {
    shared: Arc<Shared>,
    workers: usize,
}

impl BackgroundPool {
    /// A pool of `workers` threads, named `background-N`.
    pub fn new(workers: usize) -> Self {
        let workers = workers.max(1);
        let shared = Arc::new(Shared { queues: Mutex::new(Queues::default()), work: Condvar::new() });
        for index in 0..workers {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name(format!("background-{index}"))
                .spawn(move || worker_loop(&shared))
                .expect("a background worker starts");
        }
        log::info!("Background pool: {workers} worker threads");
        Self { shared, workers }
    }

    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Runs `job` on a worker and hands its result back through the [`Task`]. Dropping the task before the job starts
    /// cancels it; [`Task::detach`] lets it run unobserved.
    pub fn spawn<T: Send + 'static>(
        &self,
        priority: Priority,
        name: &'static str,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Task<T> {
        let slot = Arc::new(Mutex::new(TaskState::Pending));
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = Arc::clone(&slot);
        self.push(priority, name, Some(Arc::clone(&cancelled)), Box::new(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            *result.lock() = match outcome {
                Ok(value) => TaskState::Done(value),
                Err(_) => TaskState::Panicked,
            };
        }));
        Task { slot, cancelled, detached: false }
    }

    fn push(&self, priority: Priority, name: &'static str, cancelled: Option<Arc<AtomicBool>>, run: JobFn) {
        let mut queues = self.shared.queues.lock();
        queues.levels[priority as usize].push_back(Job { name, priority, queued_at: Instant::now(), cancelled, run });
        drop(queues);
        self.shared.work.notify_one();
    }

    /// Calls `job(index)` for every index below `count` and returns when all calls have returned. The calling thread
    /// takes parts itself, and up to one worker per remaining part joins it at the caller's priority (Visible outside
    /// the pool). The caller waits only for the parts a worker has started, so it never waits on the queue.
    pub fn for_each_index(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        if count <= 1 {
            (0..count).for_each(job);
            return;
        }
        // SAFETY: the helpers only call `job` between their `active` increment and decrement, and only while `closed`
        // is false; this function sets `closed` and then waits for `active` to drop to 0 before it returns, so no
        // helper touches `job` after its borrow ends (see `PartsBatch::help`).
        let job: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute(job) };
        let batch = Arc::new(PartsBatch {
            job,
            count,
            next: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            active: Mutex::new(0),
            idle: Condvar::new(),
        });
        let priority = RUNNING_PRIORITY.with(std::cell::Cell::get).unwrap_or(Priority::Visible);
        for _ in 0..(count - 1).min(self.workers) {
            let batch = Arc::clone(&batch);
            self.push(priority, "parallel parts", None, Box::new(move || batch.help()));
        }
        batch.work_through();
        batch.closed.store(true, Ordering::SeqCst);
        let mut active = batch.active.lock();
        while *active > 0 {
            batch.idle.wait(&mut active);
        }
    }
}

impl Drop for BackgroundPool {
    fn drop(&mut self) {
        self.shared.queues.lock().shutdown = true;
        self.shared.work.notify_all();
    }
}

fn worker_loop(shared: &Shared) {
    loop {
        let job = {
            let mut queues = shared.queues.lock();
            loop {
                if queues.shutdown {
                    return;
                }
                if let Some(job) = queues.pop_most_urgent(Instant::now()) {
                    break job;
                }
                shared.work.wait(&mut queues);
            }
        };
        if job.cancelled.as_ref().is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
            continue;
        }
        run_job(job);
    }
}

fn run_job(job: Job) {
    let waited = job.queued_at.elapsed();
    let priority = job.priority;
    if priority == Priority::Visible && waited > VISIBLE_WAIT_WARN {
        log::debug!("Background job '{}' waited {} ms in the queue", job.name, waited.as_millis());
    }
    let _span = tracing::info_span!("background", name = job.name).entered();
    RUNNING_PRIORITY.with(|running| running.set(Some(priority)));
    // `spawn` catches its job's panics to hand them to the task; this catches the rest (lanes, parallel parts).
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.run)).is_err() {
        log::error!("Background job '{}' panicked; its worker goes on", job.name);
    }
    RUNNING_PRIORITY.with(|running| running.set(None));
}

/// One `for_each_index` call's parts, shared by its caller and the workers that help.
struct PartsBatch {
    job: &'static (dyn Fn(usize) + Sync),
    count: usize,
    next: AtomicUsize,
    /// Set by the caller once every part is claimed; a helper arriving later leaves without touching `job`.
    closed: AtomicBool,
    /// Helpers inside `job`'s borrow.
    active: Mutex<usize>,
    idle: Condvar,
}

impl PartsBatch {
    fn work_through(&self) {
        loop {
            let index = self.next.fetch_add(1, Ordering::SeqCst);
            if index >= self.count {
                return;
            }
            (self.job)(index);
        }
    }

    fn help(&self) {
        {
            let mut active = self.active.lock();
            // Checked under the lock the caller waits on: once the caller sees no helper active after closing, no
            // helper can enter.
            if self.closed.load(Ordering::SeqCst) {
                return;
            }
            *active += 1;
        }
        let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.work_through()));
        let mut active = self.active.lock();
        *active -= 1;
        if *active == 0 {
            self.idle.notify_all();
        }
        drop(active);
        if let Err(payload) = finished {
            std::panic::resume_unwind(payload);
        }
    }
}

enum TaskState<T> {
    Pending,
    Done(T),
    Panicked,
    Taken,
}

/// A spawned job's result.
pub struct Task<T> {
    slot: Arc<Mutex<TaskState<T>>>,
    cancelled: Arc<AtomicBool>,
    detached: bool,
}

/// What [`Task::poll`] finds.
#[derive(Debug, PartialEq, Eq)]
pub enum TaskPoll<T> {
    Pending,
    Ready(T),
    /// The job panicked (logged), or its result was already taken.
    Failed,
}

impl<T> Task<T> {
    /// The job's result once it has run; the result is handed out once.
    pub fn poll(&mut self) -> TaskPoll<T> {
        let mut slot = self.slot.lock();
        match std::mem::replace(&mut *slot, TaskState::Taken) {
            TaskState::Pending => {
                *slot = TaskState::Pending;
                TaskPoll::Pending
            }
            TaskState::Done(value) => TaskPoll::Ready(value),
            TaskState::Panicked | TaskState::Taken => TaskPoll::Failed,
        }
    }

    pub fn is_finished(&self) -> bool {
        !matches!(*self.slot.lock(), TaskState::Pending)
    }

    /// Lets the job run though nobody waits for its result.
    pub fn detach(mut self) {
        self.detached = true;
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if !self.detached {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

type LaneJob<S> = Box<dyn FnOnce(&mut S) + Send>;

struct LaneQueues<S> {
    levels: [VecDeque<LaneJob<S>>; PRIORITY_LEVELS],
    /// A pool job is queued or running for the lane.
    scheduled: bool,
}

struct LaneShared<S> {
    name: &'static str,
    state: Mutex<S>,
    queues: Mutex<LaneQueues<S>>,
}

/// An ordered queue of jobs that share the state `S`, run one at a time on the pool: by priority, then in order.
pub struct Lane<S: Send + 'static> {
    shared: Arc<LaneShared<S>>,
}

impl<S: Send + 'static> Clone for Lane<S> {
    fn clone(&self) -> Self {
        Self { shared: Arc::clone(&self.shared) }
    }
}

impl<S: Send + 'static> Lane<S> {
    pub fn new(name: &'static str, state: S) -> Self {
        Self {
            shared: Arc::new(LaneShared {
                name,
                state: Mutex::new(state),
                queues: Mutex::new(LaneQueues { levels: Default::default(), scheduled: false }),
            }),
        }
    }

    /// Queues `job` on the lane, run on `pool`.
    pub fn submit(&self, pool: &'static BackgroundPool, priority: Priority, job: impl FnOnce(&mut S) + Send + 'static) {
        let mut queues = self.shared.queues.lock();
        queues.levels[priority as usize].push_back(Box::new(job));
        if !queues.scheduled {
            queues.scheduled = true;
            drop(queues);
            Self::schedule(&self.shared, pool, priority);
        }
    }

    /// Jobs queued and not started.
    pub fn queued(&self) -> usize {
        self.shared.queues.lock().levels.iter().map(VecDeque::len).sum()
    }

    fn schedule(shared: &Arc<LaneShared<S>>, pool: &'static BackgroundPool, priority: Priority) {
        let lane = Arc::clone(shared);
        pool.push(priority, shared.name, None, Box::new(move || Self::run_next(&lane, pool)));
    }

    /// Runs the lane's most urgent job, then queues the lane again at its next job's level, or leaves it idle.
    fn run_next(shared: &Arc<LaneShared<S>>, pool: &'static BackgroundPool) {
        let job = {
            let mut queues = shared.queues.lock();
            PRIORITIES.iter().find_map(|priority| queues.levels[*priority as usize].pop_front())
        };
        if let Some(job) = job {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut *shared.state.lock())));
            if outcome.is_err() {
                log::error!("A job of the '{}' lane panicked; the lane goes on", shared.name);
            }
        }
        let mut queues = shared.queues.lock();
        match PRIORITIES.iter().copied().find(|priority| !queues.levels[*priority as usize].is_empty()) {
            Some(next) => {
                drop(queues);
                Self::schedule(shared, pool, next);
            }
            None => queues.scheduled = false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn leaked_pool(workers: usize) -> &'static BackgroundPool {
        Box::leak(Box::new(BackgroundPool::new(workers)))
    }

    fn wait_for<T>(task: &mut Task<T>) -> TaskPoll<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match task.poll() {
                TaskPoll::Pending if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
                other => return other,
            }
        }
    }

    /// Holds the only worker until the returned sender is used.
    fn block_worker(pool: &BackgroundPool) -> mpsc::Sender<()> {
        let (release, wait) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel();
        pool.spawn(Priority::Visible, "blocker", move || {
            started.send(()).unwrap();
            let _ = wait.recv();
        })
        .detach();
        running.recv().unwrap();
        release
    }

    #[test]
    fn the_most_urgent_job_runs_first() {
        let pool = leaked_pool(1);
        let release = block_worker(pool);
        let order = Arc::new(Mutex::new(Vec::new()));
        for (priority, label) in [(Priority::Idle, "idle"), (Priority::Ahead, "ahead"), (Priority::Visible, "visible")] {
            let order = Arc::clone(&order);
            pool.spawn(priority, label, move || order.lock().push(label)).detach();
        }
        release.send(()).unwrap();
        let mut last = pool.spawn(Priority::Idle, "last", || ());
        assert_eq!(wait_for(&mut last), TaskPoll::Ready(()));
        assert_eq!(*order.lock(), ["visible", "ahead", "idle"]);
    }

    #[test]
    fn a_long_wait_promotes_a_job() {
        let mut queues = Queues::default();
        let now = Instant::now();
        let job = |queued_at| Job { name: "job", priority: Priority::Idle, queued_at, cancelled: None, run: Box::new(|| ()) };
        queues.levels[Priority::Idle as usize].push_back(job(now - PROMOTE_AFTER * 3));
        queues.levels[Priority::Visible as usize].push_back(job(now));
        let first = queues.pop_most_urgent(now).unwrap();
        assert!(first.queued_at < now, "the idle job that waited three promotions goes before a new visible one");
    }

    #[test]
    fn a_dropped_task_never_runs() {
        let pool = leaked_pool(1);
        let release = block_worker(pool);
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        drop(pool.spawn(Priority::Visible, "dropped", move || flag.store(true, Ordering::SeqCst)));
        release.send(()).unwrap();
        let mut after = pool.spawn(Priority::Idle, "after", || ());
        assert_eq!(wait_for(&mut after), TaskPoll::Ready(()));
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn a_panic_fails_its_task_and_the_worker_goes_on() {
        let pool = leaked_pool(1);
        let mut failing = pool.spawn(Priority::Visible, "panics", || -> u32 { panic!("expected by the test") });
        assert_eq!(wait_for(&mut failing), TaskPoll::Failed);
        let mut next = pool.spawn(Priority::Visible, "next", || 7);
        assert_eq!(wait_for(&mut next), TaskPoll::Ready(7));
    }

    #[test]
    fn a_lane_runs_its_jobs_in_order_one_at_a_time() {
        let pool = leaked_pool(3);
        let lane = Lane::new("test lane", (Vec::new(), 0usize));
        let concurrent = Arc::new(AtomicUsize::new(0));
        for index in 0..50 {
            let concurrent = Arc::clone(&concurrent);
            lane.submit(pool, Priority::Ahead, move |(order, _): &mut (Vec<usize>, usize)| {
                assert_eq!(concurrent.fetch_add(1, Ordering::SeqCst), 0);
                order.push(index);
                std::thread::sleep(Duration::from_micros(50));
                concurrent.fetch_sub(1, Ordering::SeqCst);
            });
        }
        let (done, finished) = mpsc::channel();
        lane.submit(pool, Priority::Ahead, move |(order, _)| done.send(order.clone()).unwrap());
        let order = finished.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(order, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn parallel_parts_finish_from_inside_a_one_worker_pool() {
        let pool = leaked_pool(1);
        let mut outer = pool.spawn(Priority::Visible, "outer", move || {
            let parts: Vec<AtomicUsize> = (0..16).map(|_| AtomicUsize::new(0)).collect();
            pool.for_each_index(parts.len(), &|index| {
                parts[index].fetch_add(1, Ordering::SeqCst);
            });
            parts.iter().all(|part| part.load(Ordering::SeqCst) == 1)
        });
        assert_eq!(wait_for(&mut outer), TaskPoll::Ready(true));
    }

    #[test]
    fn parallel_parts_run_each_index_once_on_many_workers() {
        let pool = leaked_pool(4);
        for _ in 0..200 {
            let parts: Vec<AtomicUsize> = (0..9).map(|_| AtomicUsize::new(0)).collect();
            pool.for_each_index(parts.len(), &|index| {
                parts[index].fetch_add(1, Ordering::SeqCst);
            });
            assert!(parts.iter().all(|part| part.load(Ordering::SeqCst) == 1));
        }
    }
}
