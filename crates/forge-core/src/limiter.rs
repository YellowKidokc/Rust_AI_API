//! Adaptive concurrency limiter (port of the workbench's `_system\engine\llm.py` policy):
//! a ceiling that starts at `max`; over a sliding window, if at least 10 calls were recorded and more
//! than 10% failed, the ceiling halves (min 1); after a quiet period with no halving it climbs back by one.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

pub struct AdaptiveLimiter {
    max: usize,
    window: Duration,
    recover_after: Duration,
    state: Mutex<State>,
    notify: Notify,
}

struct State {
    ceiling: usize,
    in_flight: usize,
    events: VecDeque<(Instant, bool)>,
    last_change: Instant,
}

pub struct Permit {
    limiter: Arc<AdaptiveLimiter>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.limiter.state.lock().unwrap().in_flight -= 1;
        self.limiter.notify.notify_waiters();
    }
}

impl AdaptiveLimiter {
    pub fn new(max: usize) -> Arc<Self> {
        Self::with_timing(max, Duration::from_secs(60), Duration::from_secs(60))
    }

    pub fn with_timing(max: usize, window: Duration, recover_after: Duration) -> Arc<Self> {
        let max = max.max(1);
        Arc::new(Self {
            max,
            window,
            recover_after,
            state: Mutex::new(State { ceiling: max, in_flight: 0, events: VecDeque::new(), last_change: Instant::now() }),
            notify: Notify::new(),
        })
    }

    pub fn ceiling(&self) -> usize {
        self.state.lock().unwrap().ceiling
    }

    pub fn in_flight(&self) -> usize {
        self.state.lock().unwrap().in_flight
    }

    pub async fn acquire(self: &Arc<Self>) -> Permit {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut s = self.state.lock().unwrap();
                if s.in_flight < s.ceiling {
                    s.in_flight += 1;
                    return Permit { limiter: Arc::clone(self) };
                }
            }
            notified.await;
        }
    }

    pub fn record(&self, ok: bool) {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        s.events.push_back((now, ok));
        while s.events.front().is_some_and(|(t, _)| now.duration_since(*t) > self.window) {
            s.events.pop_front();
        }
        let total = s.events.len();
        let failed = s.events.iter().filter(|(_, ok)| !ok).count();
        if total >= 10 && failed * 10 > total {
            let new = (s.ceiling / 2).max(1);
            if new != s.ceiling {
                tracing::warn!("limiter: {failed}/{total} failures in window, ceiling {} -> {new}", s.ceiling);
            }
            s.ceiling = new;
            s.events.clear();
            s.last_change = now;
        } else if s.ceiling < self.max && now.duration_since(s.last_change) >= self.recover_after {
            s.ceiling += 1;
            s.last_change = now;
            tracing::info!("limiter: recovering, ceiling -> {}", s.ceiling);
            drop(s);
            self.notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn halves_on_failures_and_recovers() {
        let l = AdaptiveLimiter::with_timing(30, Duration::from_secs(60), Duration::from_millis(30));
        for i in 0..10 {
            l.record(i % 4 != 0); // 3 of 10 fail
        }
        assert_eq!(l.ceiling(), 15);
        tokio::time::sleep(Duration::from_millis(40)).await;
        l.record(true);
        assert_eq!(l.ceiling(), 16);
    }

    #[tokio::test]
    async fn never_exceeds_ceiling() {
        let l = AdaptiveLimiter::new(3);
        let peak = Arc::new(Mutex::new(0usize));
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let l = l.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                let _p = l.acquire().await;
                let now = l.in_flight();
                {
                    let mut pk = peak.lock().unwrap();
                    *pk = (*pk).max(now);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert!(*peak.lock().unwrap() <= 3);
        assert_eq!(l.in_flight(), 0);
    }
}
