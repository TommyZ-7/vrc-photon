use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::lock::lock_unpoisoned;

/// Wakes the daemon's main loop when IPC work arrives.
#[derive(Debug, Default)]
pub struct WakeSignal {
    pending: Mutex<bool>,
    condvar: Condvar,
}

impl WakeSignal {
    pub fn notify(&self) {
        *lock_unpoisoned(&self.pending, "daemon.wake.pending") = true;
        self.condvar.notify_all();
    }

    /// Waits for a notification or the timeout, then consumes the flag.
    pub fn wait(&self, timeout: Duration) {
        let mut pending = lock_unpoisoned(&self.pending, "daemon.wake.pending");
        if !*pending {
            let (guard, _) = self
                .condvar
                .wait_timeout(pending, timeout)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending = guard;
        }
        *pending = false;
    }

    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        *lock_unpoisoned(&self.pending, "daemon.wake.pending")
    }
}

/// Wakes compression workers when a job is enqueued.
#[derive(Debug, Default)]
pub struct JobSignal {
    generation: Mutex<u64>,
    condvar: Condvar,
}

impl JobSignal {
    pub fn notify(&self) {
        let mut generation = lock_unpoisoned(&self.generation, "daemon.jobs.generation");
        *generation = generation.wrapping_add(1);
        self.condvar.notify_all();
    }

    /// Test-only view of the notification counter.
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        *lock_unpoisoned(&self.generation, "daemon.jobs.generation")
    }

    /// Returns `true` when the generation changed. Each worker keeps its own
    /// `seen` value, so one notification wakes every worker.
    pub fn wait(&self, seen: &mut u64, timeout: Duration) -> bool {
        let generation = lock_unpoisoned(&self.generation, "daemon.jobs.generation");
        if *generation != *seen {
            *seen = *generation;
            return true;
        }
        let (generation, _) = self
            .condvar
            .wait_timeout(generation, timeout)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = *generation != *seen;
        if changed {
            *seen = *generation;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn wake_signal_consumes_one_notification() {
        let signal = WakeSignal::default();
        assert!(!signal.is_pending());
        signal.notify();
        assert!(signal.is_pending());
        signal.wait(Duration::from_millis(1));
        assert!(!signal.is_pending());
    }

    #[test]
    fn job_signal_wakes_each_worker_generation() {
        let signal = JobSignal::default();
        let mut seen = 0;
        signal.notify();
        assert!(signal.wait(&mut seen, Duration::from_millis(1)));
        assert!(!signal.wait(&mut seen, Duration::from_millis(1)));
    }

    #[test]
    fn wake_signal_survives_a_poisoned_lock() {
        let signal = WakeSignal::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = signal.pending.lock().unwrap();
            panic!("poison the wake signal");
        }));
        signal.notify();
        assert!(signal.is_pending());
    }

    #[test]
    fn job_signal_survives_a_poisoned_lock() {
        let signal = JobSignal::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = signal.generation.lock().unwrap();
            panic!("poison the job signal");
        }));
        signal.notify();
        let mut seen = 0;
        assert!(signal.wait(&mut seen, Duration::from_millis(10)));
    }
}
