//! A lock-like structure that allows concurrent access to
//! the contents with very efficient cache coherency behavior.

use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{fence, AtomicU64, Ordering};

const LOCK_BIT: u64 = 1 << 63;
const LOCK_MASK: u64 = u64::MAX ^ LOCK_BIT;

pub struct OptimisticCell<T> {
    guard: AtomicU64,
    state: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for OptimisticCell<T> {}
unsafe impl<T: Sync> Sync for OptimisticCell<T> {}

pub struct OptimisticWriteGuard<'a, T> {
    cell: &'a OptimisticCell<T>,
    previous_unlocked_guard_state: u64,
}

impl<'a, T> Deref for OptimisticWriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.cell.state.get() }
    }
}

impl<'a, T> DerefMut for OptimisticWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.cell.state.get() }
    }
}

impl<'a, T> Drop for OptimisticWriteGuard<'a, T> {
    fn drop(&mut self) {
        let new_guard_state = (self.previous_unlocked_guard_state + 1) ^ LOCK_MASK;
        let old = self.cell.guard.swap(new_guard_state, Ordering::Release);
        assert_eq!(old, self.previous_unlocked_guard_state ^ LOCK_BIT);
    }
}

impl<T> OptimisticCell<T> {
    #[inline]
    fn status(&self) -> (bool, u64) {
        let guard_value = self.guard.load(Ordering::Acquire);
        let is_locked = guard_value & LOCK_BIT != 0;
        let timestamp = guard_value & LOCK_MASK;

        (is_locked, timestamp)
    }

    pub fn new(item: T) -> OptimisticCell<T> {
        OptimisticCell {
            guard: 0.into(),
            state: UnsafeCell::new(item),
        }
    }

    pub fn read(&self) -> T
    where
        T: Copy,
    {
        self.read_with(|item| *item)
    }

    pub fn read_with<R, F: Fn(&T) -> R>(&self, read_function: F) -> R
    where
        T: Copy,
    {
        loop {
            let (before_is_locked, before_timestamp) = self.status();
            if before_is_locked {
                std::hint::spin_loop();
                continue;
            }

            let state: &T = unsafe { &*self.state.get() };
            let ret = read_function(state);

            // NB: a Release fence is important for keeping the above read from
            // being reordered below this validation check.
            fence(Ordering::Release);

            let (after_is_locked, after_timestamp) = self.status();

            if after_is_locked || after_timestamp != before_timestamp {
                std::hint::spin_loop();
                continue;
            }

            return ret;
        }
    }

    pub fn lock(&self) -> OptimisticWriteGuard<'_, T> {
        loop {
            let prev = self.guard.fetch_or(LOCK_BIT, Ordering::Acquire);
            let already_locked = prev & LOCK_BIT != 0;

            if !already_locked {
                return OptimisticWriteGuard {
                    previous_unlocked_guard_state: prev,
                    cell: self,
                };
            }

            std::hint::spin_loop();
        }
    }
}

#[test]
fn concurrent_test() {
    let n: u32 = 128 * 1024 * 1024;
    let concurrency = 2;

    let cell = &OptimisticCell::new(0);
    let barrier = &std::sync::Barrier::new(concurrency as _);

    let before = std::time::Instant::now();
    std::thread::scope(|s| {
        let mut threads = vec![];
        for _ in 0..concurrency {
            let thread = s.spawn(move || {
                barrier.wait();

                for _ in 0..n {
                    let read_1 = cell.read();

                    let mut lock = cell.lock();
                    *lock += 1;
                    drop(lock);

                    let read_2 = cell.read();

                    assert_ne!(read_1, read_2);
                }
            });

            threads.push(thread);
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    dbg!(before.elapsed());
}
