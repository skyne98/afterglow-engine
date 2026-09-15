//! The RPC owner publishes a job table and waits for all tile jobs.
use maipointo::web_surface::{JOB_WORKERS, process_job};
use rayon::prelude::*;
use std::cell::OnceCell;

thread_local! {
    static POOL: OnceCell<rayon::ThreadPool> = const { OnceCell::new() };
}

pub fn initialize() -> Result<usize, String> {
    POOL.with(|slot| {
        if let Some(pool) = slot.get() { return Ok(pool.current_num_threads()); }
        let count = std::thread::available_parallelism().map_or(1, usize::from).min(JOB_WORKERS);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(count)
            .thread_name(|index| format!("paint-tile-{index}"))
            .build().map_err(|error| format!("paint tile threads: {error}"))?;
        slot.set(pool).map_err(|_| "paint tile threads already initialized".to_string())?;
        Ok(count)
    })
}

pub fn run(count: u32) {
    if count == 0 { return; }
    POOL.with(|slot| {
        let pool = slot.get().expect("paint tile threads initialized before dispatch");
        pool.install(|| {
            (0..count).into_par_iter().for_each(|job| {
                let worker = rayon::current_thread_index().expect("paint pool worker");
                process_job(job as i32, worker);
            });
        });
    });
}
