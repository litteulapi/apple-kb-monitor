//! "The keyboard sent something".

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Subscribers are called at most this often.
pub const MIN_INTERVAL: Duration = Duration::from_millis(200);

type Hook = Arc<dyn Fn() + Send + Sync>;

fn hooks() -> &'static Mutex<Vec<Hook>> {
    static H: OnceLock<Mutex<Vec<Hook>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(Vec::new()))
}

fn origin() -> Instant {
    static T: OnceLock<Instant> = OnceLock::new();
    *T.get_or_init(Instant::now)
}

/// Lets one call through per [`MIN_INTERVAL`].
#[derive(Debug, Default)]
pub struct Limiter {
    /// Milliseconds of the last call let through, plus one (0 = never).
    last: AtomicU64,
}

impl Limiter {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last: AtomicU64::new(0),
        }
    }

    /// May a call at `now_ms` (any monotonic millisecond count) go through?
    pub fn pass(&self, now_ms: u64) -> bool {
        let now = now_ms + 1;
        let last = self.last.load(Ordering::Relaxed);
        if last != 0 && now.saturating_sub(last) < crate::conv::millis_u64(MIN_INTERVAL) {
            return false;
        }
        self.last.store(now, Ordering::Relaxed);
        true
    }
}

static LIMITER: Limiter = Limiter::new();

/// Be told (with nothing but the call itself) that the keyboard is being used.
pub fn subscribe(hook: impl Fn() + Send + Sync + 'static) {
    hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Arc::new(hook));
}

/// An input report arrived.
pub fn note() {
    if !LIMITER.pass(crate::conv::millis_u64(origin().elapsed())) {
        return;
    }
    let list: Vec<Hook> = hooks()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for h in list {
        h();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_burst_is_one_call_per_interval() {
        let l = Limiter::new();
        assert!(l.pass(0), "the first one always passes");
        assert!(!l.pass(1) && !l.pass(199));
        assert!(l.pass(200));
        assert!(!l.pass(350));
        assert!(l.pass(400));
        let l = Limiter::new();
        assert_eq!((0..1250u64).filter(|i| l.pass(i * 8)).count(), 50);
    }

    #[test]
    fn subscribers_are_called_and_get_no_data() {
        let n = Arc::new(AtomicUsize::new(0));
        let c = n.clone();
        subscribe(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        std::thread::sleep(MIN_INTERVAL + Duration::from_millis(30));
        for _ in 0..500 {
            note();
        }
        let got = n.load(Ordering::SeqCst);
        assert!((1..=3).contains(&got), "{got} calls for a burst of 500");
        let _: fn() = note;
    }
}
