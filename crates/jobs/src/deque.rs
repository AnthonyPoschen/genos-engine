//! A bounded Chase-Lev work-stealing deque.
//!
//! The owner pushes and pops at the bottom. Thieves steal at the top.
//! Chase and Lev, SPAA 2005. The array does not grow: a full push fails.
//! That is the departure from their resizing circular array.

use std::mem::MaybeUninit;

#[cfg(not(feature = "loom"))]
use std::cell::UnsafeCell;
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{fence, AtomicU64, Ordering};

#[cfg(feature = "loom")]
use loom::cell::UnsafeCell;
#[cfg(feature = "loom")]
use loom::sync::atomic::{fence, AtomicU64, Ordering};

/// Outcome of a steal attempt.
#[derive(Debug)]
pub(crate) enum Stolen<T> {
    Task(T),
    Empty,
    Retry,
}

/// One owner's deque. `Send + Sync` when `T: Send`.
pub(crate) struct Deque<T> {
    bottom: AtomicU64,
    top: AtomicU64,
    mask: u64,
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

impl<T> Deque<T> {
    /// `capacity` is rounded up to a power of two, at least 2.
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two().max(2);
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(UnsafeCell::new(MaybeUninit::uninit()));
        }
        Self {
            bottom: AtomicU64::new(0),
            top: AtomicU64::new(0),
            mask: u64::try_from(capacity - 1).unwrap_or(1),
            buffer: buffer.into_boxed_slice(),
        }
    }

    fn index(&self, raw: u64) -> usize {
        usize::try_from(raw & self.mask).unwrap_or(0)
    }

    /// Push as the owner. `false` means the deque is full.
    pub(crate) fn push(&self, task: T) -> bool {
        let bottom = self.bottom.load(Ordering::Relaxed);
        let top = self.top.load(Ordering::Acquire);
        if bottom.wrapping_sub(top) >= u64::try_from(self.buffer.len()).unwrap_or(u64::MAX) {
            return false;
        }
        let index = self.index(bottom);
        let Some(cell) = self.buffer.get(index) else {
            return false;
        };
        // SAFETY: this slot is not in `[top, bottom)`. The owner is the only
        // pusher, so no thief reads it until `bottom` is stored below.
        unsafe {
            write_slot(cell, task);
        }
        self.bottom.store(bottom.wrapping_add(1), Ordering::Release);
        true
    }

    /// Pop the newest task. Owner only.
    pub(crate) fn pop(&self) -> Option<T> {
        let bottom = self.bottom.load(Ordering::Relaxed);
        let top = self.top.load(Ordering::Acquire);
        if bottom == top || bottom == 0 {
            return None;
        }
        let next = bottom - 1;
        self.bottom.store(next, Ordering::Relaxed);
        fence(Ordering::SeqCst);
        let top = self.top.load(Ordering::Relaxed);
        if top > next {
            self.bottom.store(top, Ordering::Relaxed);
            return None;
        }
        let index = self.index(next);
        let Some(cell) = self.buffer.get(index) else {
            self.bottom.store(bottom, Ordering::Relaxed);
            return None;
        };
        // SAFETY: `next` was in `[top, bottom)` after the fence, so `push`
        // initialized this slot. On the last item the CAS below decides which
        // caller keeps the copy. A failed CAS forgets this copy.
        let task = unsafe { read_slot(cell) };
        if top == next {
            if self
                .top
                .compare_exchange(
                    top,
                    top.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_err()
            {
                std::mem::forget(task);
                self.bottom.store(top.wrapping_add(1), Ordering::Relaxed);
                return None;
            }
            self.bottom.store(top.wrapping_add(1), Ordering::Relaxed);
        }
        Some(task)
    }

    /// Steal the oldest task. `Retry` means another thief or the owner won the race.
    pub(crate) fn steal(&self) -> Stolen<T> {
        let top = self.top.load(Ordering::Acquire);
        fence(Ordering::SeqCst);
        let bottom = self.bottom.load(Ordering::Acquire);
        if top >= bottom {
            return Stolen::Empty;
        }
        let index = self.index(top);
        let Some(cell) = self.buffer.get(index) else {
            return Stolen::Empty;
        };
        // SAFETY: `top < bottom`, so the owner has published this slot. The
        // copy is kept only if the CAS wins. A loser forgets it so the winner
        // is the sole owner.
        let task = unsafe { read_slot(cell) };
        if self
            .top
            .compare_exchange(
                top,
                top.wrapping_add(1),
                Ordering::SeqCst,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Stolen::Task(task)
        } else {
            std::mem::forget(task);
            Stolen::Retry
        }
    }
}

/// Read an initialized slot.
///
/// # Safety
///
/// `cell` is initialized, and this caller is the only one that keeps the copy.
unsafe fn read_slot<T>(cell: &UnsafeCell<MaybeUninit<T>>) -> T {
    #[cfg(not(feature = "loom"))]
    {
        cell.get().read().assume_init()
    }
    #[cfg(feature = "loom")]
    {
        cell.with(|ptr| ptr.read().assume_init())
    }
}

/// Write a slot the caller owns.
///
/// # Safety
///
/// `cell` is not concurrently read or written.
unsafe fn write_slot<T>(cell: &UnsafeCell<MaybeUninit<T>>, task: T) {
    #[cfg(not(feature = "loom"))]
    {
        (*cell.get()).write(task);
    }
    #[cfg(feature = "loom")]
    {
        cell.with_mut(|ptr| (*ptr).write(task));
    }
}

impl<T> Drop for Deque<T> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

// SAFETY: the owner thread is the only caller of `push` and `pop`. Thieves
// only call `steal`. The atomics publish the indices that make each slot's
// read and write exclusive. `T: Send` allows the value to move to the thief.
unsafe impl<T: Send> Send for Deque<T> {}
// SAFETY: same as `Send`. Thieves share the deque through `&Deque`.
unsafe impl<T: Send> Sync for Deque<T> {}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::{Deque, Stolen};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn the_owner_pops_what_it_pushed_newest_first() {
        let deque = Deque::new(8);
        assert!(deque.push(1));
        assert!(deque.push(2));
        assert_eq!(deque.pop(), Some(2));
        assert_eq!(deque.pop(), Some(1));
        assert_eq!(deque.pop(), None);
    }

    #[test]
    fn a_full_deque_refuses_another_push() {
        let deque = Deque::new(2);
        assert!(deque.push(1));
        assert!(deque.push(2));
        assert!(!deque.push(3));
        assert_eq!(deque.pop(), Some(2));
        assert!(deque.push(3));
    }

    #[test]
    fn thieves_take_every_task_once() {
        let deque = Arc::new(Deque::new(128));
        for item in 0..100u32 {
            assert!(deque.push(item));
        }
        let seen = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let deque = Arc::clone(&deque);
            let seen = Arc::clone(&seen);
            handles.push(thread::spawn(move || {
                let mut got = Vec::new();
                loop {
                    match deque.steal() {
                        Stolen::Task(item) => {
                            seen.fetch_add(1, Ordering::Relaxed);
                            got.push(item);
                        }
                        Stolen::Empty => break,
                        Stolen::Retry => {}
                    }
                }
                got
            }));
        }
        let mut all = Vec::new();
        for handle in handles {
            all.extend(handle.join().unwrap());
        }
        while let Some(item) = deque.pop() {
            seen.fetch_add(1, Ordering::Relaxed);
            all.push(item);
        }
        all.sort_unstable();
        assert_eq!(all, (0..100).collect::<Vec<_>>());
        assert_eq!(seen.load(Ordering::Relaxed), 100);
    }
}

#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::{Deque, Stolen};
    use loom::sync::Arc;
    use loom::thread;

    fn collect(popped: Option<u32>, stolen: Stolen<u32>) -> Vec<u32> {
        let mut got = Vec::new();
        if let Some(value) = popped {
            got.push(value);
        }
        if let Stolen::Task(value) = stolen {
            got.push(value);
        }
        got
    }

    #[test]
    fn the_last_element_is_taken_once() {
        loom::model(|| {
            let deque = Arc::new(Deque::new(2));
            assert!(deque.push(1));
            let owner = Arc::clone(&deque);
            let thief = Arc::clone(&deque);
            let pop = thread::spawn(move || owner.pop());
            let steal = thread::spawn(move || thief.steal());
            let mut got = collect(pop.join().unwrap(), steal.join().unwrap());
            if let Some(value) = deque.pop() {
                got.push(value);
            }
            got.sort_unstable();
            assert_eq!(got, vec![1]);
        });
    }

    #[test]
    fn push_pop_and_steal_cover_two_elements() {
        loom::model(|| {
            let deque = Arc::new(Deque::new(2));
            assert!(deque.push(1));
            assert!(deque.push(2));
            let owner = Arc::clone(&deque);
            let thief = Arc::clone(&deque);
            let pop = thread::spawn(move || owner.pop());
            let steal = thread::spawn(move || thief.steal());
            let mut got = collect(pop.join().unwrap(), steal.join().unwrap());
            if let Some(value) = deque.pop() {
                got.push(value);
            }
            got.sort_unstable();
            assert_eq!(got, vec![1, 2]);
        });
    }
}
