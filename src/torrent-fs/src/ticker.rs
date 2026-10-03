use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// More than one: a torrent between `start` and its first live stats is not live yet.
const QUIET_TICKS_BEFORE_STOPPING: u32 = 5;

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
/// Background work in flight; shutdown waits for it before the library is unloaded.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// So shutdown does not wait out a whole tick.
const SLICE: std::time::Duration = std::time::Duration::from_millis(100);

fn rest(how_long: std::time::Duration) {
    let mut left = how_long;
    while left > std::time::Duration::ZERO && !STOPPING.load(Ordering::Relaxed) {
        let slice = left.min(SLICE);
        std::thread::sleep(slice);
        left -= slice;
    }
}

pub fn took_on_work() {
    BUSY.fetch_add(1, Ordering::SeqCst);
}

pub fn finished_work() {
    BUSY.fetch_sub(1, Ordering::SeqCst);
}

pub fn still_busy() -> usize {
    BUSY.load(Ordering::SeqCst)
}

pub fn settle(within: std::time::Duration) {
    STOPPING.store(true, Ordering::SeqCst);
    let until = std::time::Instant::now() + within;
    while (is_running() || still_busy() > 0) && std::time::Instant::now() < until {
        std::thread::sleep(SLICE);
    }
    STOPPING.store(false, Ordering::SeqCst);
}

pub fn should_keep_ticking(anything_live: bool, quiet_ticks: u32) -> bool {
    anything_live || quiet_ticks < QUIET_TICKS_BEFORE_STOPPING
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

pub fn wake() {
    if STOPPING.load(Ordering::Relaxed) {
        return;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut quiet_ticks = 0u32;
        loop {
            rest(EVERY);
            if STOPPING.load(Ordering::Relaxed) {
                break;
            }
            let live = crate::torrent_session::anything_live();
            quiet_ticks = if live { 0 } else { quiet_ticks + 1 };
            if !should_keep_ticking(live, quiet_ticks) {
                break;
            }
            if live {
                crate::listing_moved_on();
            }
            // Unconditional: the tick that finds nothing running takes the indicator off.
            crate::show_what_is_downloading();
            crate::downloads_moved_on();
        }
        crate::show_what_is_downloading();
        RUNNING.store(false, Ordering::SeqCst);
        // Race: a torrent started while the loop was giving up would be left without a ticker.
        if !STOPPING.load(Ordering::Relaxed) && crate::torrent_session::anything_live() {
            wake();
        }
    });
}

/// Tests run in parallel and share one ticker, so those touching it take turns.
#[cfg(test)]
pub(crate) fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    HELD.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn something_running_always_keeps_the_ticker_going() {
        assert!(should_keep_ticking(true, 0));
        assert!(should_keep_ticking(true, 99));
    }

    #[test]
    fn a_quiet_moment_is_waited_out_rather_than_given_up_on() {
        assert!(should_keep_ticking(false, 1));
        assert!(should_keep_ticking(false, QUIET_TICKS_BEFORE_STOPPING - 1));
    }

    #[test]
    fn nothing_running_for_long_enough_stops_the_thread() {
        assert!(!should_keep_ticking(false, QUIET_TICKS_BEFORE_STOPPING));
        assert!(!should_keep_ticking(false, 100));
    }

    #[test]
    fn asking_twice_does_not_start_a_second_ticker() {
        let _held = super::one_at_a_time();
        wake();
        assert!(is_running());
        wake();
        assert!(is_running());
        settle(std::time::Duration::from_secs(2));
        assert!(!is_running());
    }

    #[test]
    fn shutting_down_waits_for_the_ticker_rather_than_leaving_it_running() {
        let _held = super::one_at_a_time();
        wake();
        assert!(is_running());
        let started = std::time::Instant::now();
        settle(std::time::Duration::from_secs(3));
        assert!(
            !is_running(),
            "a thread left running in an unloaded library is a crash"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "it should stop when asked, not time out"
        );
    }

    #[test]
    fn nothing_new_starts_once_shutdown_has_begun() {
        let _held = super::one_at_a_time();
        STOPPING.store(true, Ordering::SeqCst);
        let before = is_running();
        wake();
        assert_eq!(is_running(), before, "a late wake-up is refused");
        STOPPING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn work_put_on_a_thread_is_counted_until_it_is_done() {
        let _held = super::one_at_a_time();
        let before = still_busy();
        took_on_work();
        assert_eq!(still_busy(), before + 1);
        finished_work();
        assert_eq!(still_busy(), before);
    }
}
