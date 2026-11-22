//! Where the time goes (docs/REGENERATE.md §1): with `DOCAIR_TIMING=1`, each timed step says on
//! stderr how long it took. Without it, a timed step costs one look at a flag.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Whether steps are timed: `DOCAIR_TIMING` set to anything but `0` or empty.
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("DOCAIR_TIMING")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0")
    })
}

/// A step being timed: it says how long it took when it ends (when it is dropped).
pub struct Stopwatch {
    name: &'static str,
    started: Option<Instant>,
}

/// Time a step, from now until the returned value is dropped.
pub fn stopwatch(name: &'static str) -> Stopwatch {
    Stopwatch {
        name,
        started: on().then(Instant::now),
    }
}

impl Drop for Stopwatch {
    fn drop(&mut self) {
        if let Some(t) = self.started {
            say(self.name, t.elapsed());
        }
    }
}

fn say(name: &str, d: Duration) {
    eprintln!("docair timing: {name}: {:.1} ms", d.as_secs_f64() * 1000.0);
}

/// Time spent on one kind of work across threads, added up: said by `report`.
#[derive(Default)]
pub struct Sum {
    nanos: AtomicU64,
    count: AtomicU64,
}

impl Sum {
    /// Nothing added up yet; usable in a `static`.
    pub const fn new() -> Self {
        Sum {
            nanos: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Run `f`, and add the time it took.
    pub fn time<R>(&self, f: impl FnOnce() -> R) -> R {
        if !on() {
            return f();
        }
        let t = Instant::now();
        let r = f();
        self.nanos
            .fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        r
    }

    /// Say the added-up time and how many times it was spent.
    pub fn report(&self, name: &str) {
        if on() {
            let n = self.count.load(Ordering::Relaxed);
            let d = Duration::from_nanos(self.nanos.load(Ordering::Relaxed));
            eprintln!(
                "docair timing: {name}: {:.1} ms in all, {n} times",
                d.as_secs_f64() * 1000.0
            );
        }
    }
}
