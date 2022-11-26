# optimistic-cell

<a href="https://docs.rs/optimistic-cell"><img src="https://docs.rs/optimistic-cell/badge.svg"></a>

A highly cache-efficient lock-like container for working with concurrent data where
reads may take place optimistically and without modifying any cachelines.

Due to the fact that reads may access data that races with writes and is
only validated later, only items that are marked as `Copy` may be read optimistically.

The write guard is essentially just a plain spinlock.

```rust
let n: u32 = 128 * 1024 * 1024;
let concurrency = 4;

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
```

Workload scalability is highly dependent on the frequency of writes, and the
duration that a writer may hold the cell locked for before dropping it.
