//! Cook work across cores with `std::thread::scope` (the repo has no job system
//! yet; ADR 0031 is only proposed).
//!
//! Each item is computed on its own and its result lands at its own index, so
//! the output is the same for any thread count. Only the order the work is
//! picked up in changes.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// How many threads a cook may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threads(pub NonZeroUsize);

impl Threads {
    /// Every core the system reports.
    #[must_use]
    pub fn all() -> Self {
        Self(std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN))
    }

    /// One thread: the work runs in order on the caller.
    pub const ONE: Self = Self(NonZeroUsize::MIN);

    #[must_use]
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl Default for Threads {
    fn default() -> Self {
        Self::all()
    }
}

/// `f` over `items`, results in item order. Items are taken largest `cost`
/// first so one big item does not start last.
pub fn map_by_cost<T: Sync, R: Send>(
    items: &[T],
    threads: Threads,
    cost: impl Fn(&T) -> usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let workers = threads.get().min(items.len());
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(cost(&items[i])));
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some(&i) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let r = f(&items[i]);
                    if let Ok(mut slot) = slots[i].lock() {
                        *slot = Some(r);
                    }
                }
            });
        }
    });
    slots
        .into_iter()
        .filter_map(|s| s.into_inner().ok().flatten())
        .collect()
}
