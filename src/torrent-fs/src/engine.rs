//! One runtime for the plugin's lifetime: a runtime built per action cancels every task librqbit spawned.

use std::sync::Mutex;
use std::time::Duration;

fn slot() -> &'static Mutex<Option<tokio::runtime::Runtime>> {
    static HELD: std::sync::OnceLock<Mutex<Option<tokio::runtime::Runtime>>> =
        std::sync::OnceLock::new();
    HELD.get_or_init(|| Mutex::new(None))
}

fn held() -> std::sync::MutexGuard<'static, Option<tokio::runtime::Runtime>> {
    slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn handle() -> Option<tokio::runtime::Handle> {
    let mut slot = held();
    if slot.is_none() {
        let built = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ic-torrent")
            .build();
        match built {
            Ok(runtime) => *slot = Some(runtime),
            Err(why) => {
                crate::note(&format!("no runtime for the torrent session: {why}"));
                return None;
            }
        }
    }
    slot.as_ref().map(|runtime| runtime.handle().clone())
}

pub fn is_running() -> bool {
    held().is_some()
}

/// Must run before unload, or tokio's worker threads keep running in freed code.
pub fn stop(within: Duration) {
    let taken = held().take();
    if let Some(runtime) = taken {
        runtime.shutdown_timeout(within);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_is_the_same_one_every_time_it_is_asked_for() {
        let _held = crate::ticker::one_at_a_time();
        let first = handle().expect("a runtime");
        let second = handle().expect("a runtime");
        assert_eq!(first.id(), second.id());
        stop(Duration::from_secs(1));
    }

    #[test]
    fn work_left_running_survives_the_call_that_started_it() {
        let _held = crate::ticker::one_at_a_time();
        let runtime = handle().expect("a runtime");
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink = seen.clone();
        runtime.block_on(async {
            let sink = sink.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                sink.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            "the runtime went away and took the work with it"
        );
        stop(Duration::from_secs(1));
    }

    #[test]
    fn stopping_it_twice_is_harmless() {
        let _held = crate::ticker::one_at_a_time();
        let _ = handle();
        stop(Duration::from_secs(1));
        assert!(!is_running());
        stop(Duration::from_secs(1));
        assert!(!is_running());
    }
}
