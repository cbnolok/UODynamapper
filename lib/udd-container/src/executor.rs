//! Where a payload's independent parts decode in parallel. Tools keep the default, a rayon pool of its own; an
//! application with its own scheduler (Dynamapper) sets its executor once, before the first parallel decode, so the
//! decodes share its threads instead of starting more.

use std::sync::OnceLock;

/// Runs independent jobs, in parallel where it can.
pub trait ParallelExecutor: Send + Sync {
    /// Calls `job(index)` once for every index below `count`, on any threads, and returns when every call has
    /// returned. The calling thread may run some of the calls itself.
    fn for_each_index(&self, count: usize, job: &(dyn Fn(usize) + Sync));
}

/// Runs every job on the calling thread, in order.
pub struct SequentialExecutor;

impl ParallelExecutor for SequentialExecutor {
    fn for_each_index(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        (0..count).for_each(job);
    }
}

/// The most threads the default executor starts. More would take cores from the other work for a few milliseconds
/// less on a page.
const RAYON_EXECUTOR_THREADS_MAX: usize = 8;

/// The default executor: a rayon pool of its own, so its cap holds whatever else uses rayon's global pool, started on
/// the first parallel decode.
pub struct RayonExecutor;

impl RayonExecutor {
    fn pool() -> &'static rayon::ThreadPool {
        static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
        POOL.get_or_init(|| {
            let threads =
                std::thread::available_parallelism().map_or(1, |threads| threads.get()).min(RAYON_EXECUTOR_THREADS_MAX);
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|index| format!("uddp-frame-decode-{index}"))
                .build()
                .expect("the frame decode pool starts")
        })
    }
}

impl ParallelExecutor for RayonExecutor {
    fn for_each_index(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        use rayon::prelude::*;
        Self::pool().install(|| (0..count).into_par_iter().for_each(job));
    }
}

static EXECUTOR: OnceLock<Box<dyn ParallelExecutor>> = OnceLock::new();

/// Sets the executor every parallel decode runs on. It can be set once, before the first parallel decode; afterwards
/// the executor in use stays and `executor` comes back.
pub fn set_parallel_executor(executor: Box<dyn ParallelExecutor>) -> Result<(), Box<dyn ParallelExecutor>> {
    EXECUTOR.set(executor)
}

/// The executor parallel decodes run on: the one set, else `RayonExecutor`.
pub(crate) fn parallel_executor() -> &'static dyn ParallelExecutor {
    EXECUTOR.get_or_init(|| Box::new(RayonExecutor)).as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn runs_every_index_once(executor: &dyn ParallelExecutor) {
        const JOBS: usize = 37;
        let calls: Vec<AtomicUsize> = (0..JOBS).map(|_| AtomicUsize::new(0)).collect();
        executor.for_each_index(JOBS, &|index| {
            calls[index].fetch_add(1, Ordering::Relaxed);
        });
        assert!(calls.iter().all(|calls| calls.load(Ordering::Relaxed) == 1));
    }

    #[test]
    fn sequential_and_rayon_executors_run_every_index_once() {
        runs_every_index_once(&SequentialExecutor);
        runs_every_index_once(&RayonExecutor);
    }
}
