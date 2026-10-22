//! How long a map keeps its players' movement before it sends it on (the
//! cork's window, `[network] broadcast_batch_*`), steered by the map's own
//! load.
//!
//! A long window costs every client its length in latency and saves the
//! map's thread a push per client and per window; a short one the other
//! way round. So each map keeps its own, between `broadcast_batch_min_ms`
//! and `broadcast_batch_max_ms`, starting at `broadcast_batch_ms`: the
//! share of its thread's time its updates and events take (as the stats
//! file measures it), smoothed over about a second, above
//! `broadcast_batch_busy_high` doubles the window, below
//! `broadcast_batch_busy_low` halves it, and between holds it. A mailbox
//! deeper than `broadcast_batch_mailbox` (smoothed the same way) counts as
//! busy whatever the share. After a change the window holds for a smoothing
//! time, so that what the change did shows before the next.

use std::time::{Duration, Instant};

/// The smoothing's time constant, and the least time between changes.
pub const SMOOTHING: Duration = Duration::from_secs(1);

/// The window's settings (`[network] broadcast_batch_*`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchConfig {
    /// The window a map starts at (`broadcast_batch_ms`).
    pub start: Duration,
    pub min: Duration,
    pub max: Duration,
    /// The busy shares past which the window doubles, and under which it
    /// halves.
    pub busy_high: f64,
    pub busy_low: f64,
    /// Events waiting past which the map counts as busy (0: never).
    pub mailbox: usize,
}

impl BatchConfig {
    /// A window that doesn't change.
    pub fn fixed(window: Duration) -> Self {
        Self {
            start: window,
            min: window,
            max: window,
            ..Self::default()
        }
    }

    /// From the config.
    pub fn from_config(n: &crate::config::NetworkConfig) -> Self {
        let ms = Duration::from_millis;
        let min = ms(n.broadcast_batch_min_ms);
        Self {
            start: ms(n.broadcast_batch_ms),
            min,
            max: ms(n.broadcast_batch_max_ms).max(min),
            busy_high: n.broadcast_batch_busy_high,
            busy_low: n.broadcast_batch_busy_low.min(n.broadcast_batch_busy_high),
            mailbox: n.broadcast_batch_mailbox,
        }
    }
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self::from_config(&crate::config::NetworkConfig::default())
    }
}

/// A map's window, and what steers it.
#[derive(Debug, Clone)]
pub struct BatchWindow {
    window: Duration,
    /// The smoothed busy share and mailbox, none before the first look.
    busy: Option<f64>,
    mailbox: f64,
    /// When it was last looked at, and last changed.
    seen: Option<Instant>,
    changed: Option<Instant>,
    /// The thread's time taken since it was last looked at.
    taken: Duration,
}

impl BatchWindow {
    pub fn new(cfg: &BatchConfig) -> Self {
        Self {
            window: cfg.start.clamp(cfg.min, cfg.max),
            busy: None,
            mailbox: 0.0,
            seen: None,
            changed: None,
            taken: Duration::ZERO,
        }
    }

    /// The window now.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// The smoothed busy share (0 before the first look).
    pub fn busy(&self) -> f64 {
        self.busy.unwrap_or(0.0)
    }

    /// The map's thread worked for `took` (an update, a burst of events, a
    /// push).
    pub fn took(&mut self, took: Duration) {
        self.taken += took;
    }

    /// Look at the load at `now`, with `mailbox` events waiting: the time
    /// taken since the last look over the time gone by, smoothed, and the
    /// window changed if it should be. Once an update.
    pub fn look(&mut self, cfg: &BatchConfig, now: Instant, mailbox: usize) {
        let taken = std::mem::take(&mut self.taken);
        let Some(seen) = self.seen.replace(now) else {
            self.changed = Some(now);
            self.window = self.window.clamp(cfg.min, cfg.max);
            return;
        };
        let dt = now.saturating_duration_since(seen);
        if dt.is_zero() {
            self.taken = taken;
            self.seen = Some(seen);
            return;
        }
        let share = (taken.as_secs_f64() / dt.as_secs_f64()).min(1.0);
        let a = 1.0 - (-dt.as_secs_f64() / SMOOTHING.as_secs_f64()).exp();
        let busy = match self.busy {
            Some(b) => b + a * (share - b),
            None => share,
        };
        self.busy = Some(busy);
        self.mailbox += a * (mailbox as f64 - self.mailbox);
        // A reload's bounds hold at once.
        let mut window = self.window.clamp(cfg.min, cfg.max);
        let settled = self
            .changed
            .is_none_or(|c| now.saturating_duration_since(c) >= SMOOTHING);
        if settled {
            let deep = cfg.mailbox > 0 && self.mailbox > cfg.mailbox as f64;
            if busy > cfg.busy_high || deep {
                window = (window * 2).max(Duration::from_millis(1)).min(cfg.max);
            } else if busy < cfg.busy_low {
                // In whole milliseconds, as it is set.
                let half = Duration::from_millis((window.as_millis() / 2) as u64);
                window = half.max(cfg.min);
            }
        }
        if window != self.window {
            self.window = window;
            self.changed = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BatchConfig {
        BatchConfig {
            start: Duration::from_millis(10),
            min: Duration::from_millis(2),
            max: Duration::from_millis(40),
            busy_high: 0.7,
            busy_low: 0.4,
            mailbox: 1000,
        }
    }

    const TICK: Duration = Duration::from_millis(50);

    /// `secs` of updates, each `busy` of its 50 ms taken and `mailbox`
    /// waiting: the window after each.
    fn run(
        w: &mut BatchWindow,
        cfg: &BatchConfig,
        now: &mut Instant,
        secs: f64,
        busy: f64,
        mailbox: usize,
    ) -> Vec<Duration> {
        let n = (secs / TICK.as_secs_f64()).round() as usize;
        (0..n)
            .map(|_| {
                *now += TICK;
                w.took(TICK.mul_f64(busy));
                w.look(cfg, *now, mailbox);
                w.window()
            })
            .collect()
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn it_starts_at_its_start_within_its_bounds() {
        assert_eq!(BatchWindow::new(&cfg()).window(), ms(10));
        let low = BatchConfig {
            start: ms(1),
            ..cfg()
        };
        assert_eq!(BatchWindow::new(&low).window(), ms(2));
        let high = BatchConfig {
            start: ms(90),
            ..cfg()
        };
        assert_eq!(BatchWindow::new(&high).window(), ms(40));
    }

    #[test]
    fn sustained_load_doubles_it_to_its_max_a_step_a_second() {
        let (c, mut now) = (cfg(), Instant::now());
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        let trace = run(&mut w, &c, &mut now, 6.0, 0.9, 0);
        // The smoothing takes a while to cross 70%; then a step a second.
        let changes: Vec<Duration> = trace
            .windows(2)
            .filter(|p| p[0] != p[1])
            .map(|p| p[1])
            .collect();
        assert_eq!(changes, [ms(20), ms(40)]);
        let first = trace.iter().position(|d| *d == ms(20)).unwrap();
        let second = trace.iter().position(|d| *d == ms(40)).unwrap();
        assert!(second - first >= 20, "a second between steps");
        assert_eq!(*trace.last().unwrap(), ms(40));
    }

    #[test]
    fn it_halves_to_its_min_as_the_load_eases() {
        let (c, mut now) = (cfg(), Instant::now());
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        run(&mut w, &c, &mut now, 6.0, 0.9, 0);
        assert_eq!(w.window(), ms(40));
        let trace = run(&mut w, &c, &mut now, 8.0, 0.1, 0);
        let changes: Vec<Duration> = trace
            .windows(2)
            .filter(|p| p[0] != p[1])
            .map(|p| p[1])
            .collect();
        assert_eq!(changes, [ms(20), ms(10), ms(5), ms(2)]);
        assert_eq!(w.window(), ms(2));
    }

    #[test]
    fn between_the_thresholds_it_holds() {
        let (c, mut now) = (cfg(), Instant::now());
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        let trace = run(&mut w, &c, &mut now, 20.0, 0.55, 0);
        assert!(trace.iter().all(|d| *d == ms(10)));
        // Swinging between 45% and 65% each half second: held too.
        for _ in 0..10 {
            for busy in [0.45, 0.65] {
                assert!(
                    run(&mut w, &c, &mut now, 0.5, busy, 0)
                        .iter()
                        .all(|d| *d == ms(10))
                );
            }
        }
    }

    #[test]
    fn a_deep_mailbox_counts_as_busy() {
        let (c, mut now) = (cfg(), Instant::now());
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        run(&mut w, &c, &mut now, 5.0, 0.5, 5000);
        assert_eq!(w.window(), ms(40));
        let off = BatchConfig { mailbox: 0, ..c };
        let mut w = BatchWindow::new(&off);
        w.look(&off, now, 0);
        run(&mut w, &off, &mut now, 5.0, 0.5, 5000);
        assert_eq!(w.window(), ms(10));
    }

    #[test]
    fn never_past_its_bounds() {
        let (c, mut now) = (cfg(), Instant::now());
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        for (secs, busy) in [(30.0, 1.0), (30.0, 0.0), (3.0, 0.95), (10.0, 0.2)] {
            for d in run(&mut w, &c, &mut now, secs, busy, 0) {
                assert!((c.min..=c.max).contains(&d), "{d:?}");
            }
        }
        // A reload's narrower bounds hold at the next look.
        let narrow = BatchConfig {
            min: ms(15),
            max: ms(25),
            ..c
        };
        now += TICK;
        w.look(&narrow, now, 0);
        assert!((ms(15)..=ms(25)).contains(&w.window()));
        // A min of 0 is reached by halving and left by doubling.
        let zero = BatchConfig { min: ms(0), ..c };
        let mut w = BatchWindow::new(&zero);
        w.look(&zero, now, 0);
        run(&mut w, &zero, &mut now, 10.0, 0.0, 0);
        assert_eq!(w.window(), ms(0));
        run(&mut w, &zero, &mut now, 4.0, 1.0, 0);
        assert!(w.window() > ms(0));
    }

    #[test]
    fn fixed_never_moves() {
        let c = BatchConfig::fixed(ms(20));
        let mut now = Instant::now();
        let mut w = BatchWindow::new(&c);
        w.look(&c, now, 0);
        for busy in [1.0, 0.0] {
            assert!(
                run(&mut w, &c, &mut now, 10.0, busy, 100_000)
                    .iter()
                    .all(|d| *d == ms(20))
            );
        }
    }
}
