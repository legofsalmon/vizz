//! Health monitoring: frame timing, memory, and CPU for the running app.
//!
//! One [`HealthMonitor`] lives on the render thread. Call [`HealthMonitor::on_frame`]
//! once per frame with the measured frame time; call [`HealthMonitor::snapshot`]
//! whenever you want numbers (HUD overlay, periodic log line, end-of-run
//! benchmark report). Process memory/CPU are sampled lazily and rate-limited
//! so snapshots are cheap enough to take every frame.
//!
//! [`HealthSnapshot`] is `serde::Serialize`, which is what makes it double as
//! the benchmark artifact: headless runs dump it as JSON, and CI can diff
//! reports across commits to catch performance regressions.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// How the monitor judges frames.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Frame budget; frames slower than this count as "over budget".
    /// Defaults to 60 fps (16.67 ms).
    pub frame_budget: Duration,
    /// How many recent frames the rolling window keeps (percentiles, fps).
    pub window: usize,
    /// Minimum interval between process memory/CPU refreshes.
    pub sys_refresh_interval: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            frame_budget: Duration::from_nanos(16_666_667),
            window: 600, // 10 seconds at 60 fps
            sys_refresh_interval: Duration::from_millis(500),
        }
    }
}

/// A point-in-time health report. Serializable → doubles as benchmark output.
#[derive(Debug, Clone, Serialize)]
pub struct HealthSnapshot {
    pub uptime_s: f64,
    pub frames_total: u64,
    /// Effective fps over the rolling window (from summed frame times).
    pub fps: f32,
    pub frame_avg_ms: f32,
    pub frame_p95_ms: f32,
    pub frame_p99_ms: f32,
    pub frame_worst_ms: f32,
    /// Percentage of frames in the window that blew the budget.
    pub over_budget_window_pct: f32,
    /// Total over-budget frames since startup.
    pub over_budget_total: u64,
    /// Resident set size of this process, in MiB. `None` if unavailable.
    pub rss_mib: Option<f64>,
    /// CPU usage of this process in percent of one core. `None` if unavailable.
    pub cpu_pct: Option<f32>,
}

impl HealthSnapshot {
    /// One-line human-readable form, shared by the periodic log and (later)
    /// the GUI HUD so both always agree.
    pub fn log_line(&self) -> String {
        format!(
            "fps {:5.1} | frame avg {:5.2}ms p95 {:5.2}ms p99 {:5.2}ms worst {:5.2}ms | over-budget {:4.1}% (total {}) | rss {} | cpu {}",
            self.fps,
            self.frame_avg_ms,
            self.frame_p95_ms,
            self.frame_p99_ms,
            self.frame_worst_ms,
            self.over_budget_window_pct,
            self.over_budget_total,
            self.rss_mib
                .map(|m| format!("{m:.0} MiB"))
                .unwrap_or_else(|| "n/a".into()),
            self.cpu_pct
                .map(|c| format!("{c:.0}%"))
                .unwrap_or_else(|| "n/a".into()),
        )
    }
}

pub struct HealthMonitor {
    cfg: HealthConfig,
    start: Instant,
    frames_ms: VecDeque<f32>,
    frames_total: u64,
    over_budget_total: u64,
    sys: System,
    pid: Option<sysinfo::Pid>,
    last_sys_refresh: Option<Instant>,
    cached_rss_mib: Option<f64>,
    cached_cpu_pct: Option<f32>,
}

impl HealthMonitor {
    pub fn new(cfg: HealthConfig) -> Self {
        Self {
            cfg,
            start: Instant::now(),
            frames_ms: VecDeque::new(),
            frames_total: 0,
            over_budget_total: 0,
            sys: System::new(),
            pid: sysinfo::get_current_pid().ok(),
            last_sys_refresh: None,
            cached_rss_mib: None,
            cached_cpu_pct: None,
        }
    }

    pub fn config(&self) -> &HealthConfig {
        &self.cfg
    }

    /// Record one frame's duration.
    pub fn on_frame(&mut self, frame_time: Duration) {
        let ms = frame_time.as_secs_f32() * 1e3;
        if self.frames_ms.len() == self.cfg.window {
            self.frames_ms.pop_front();
        }
        self.frames_ms.push_back(ms);
        self.frames_total += 1;
        if frame_time > self.cfg.frame_budget {
            self.over_budget_total += 1;
        }
    }

    /// Produce a report. Cheap: percentiles sort at most `window` floats and
    /// the sysinfo refresh is rate-limited by `sys_refresh_interval`.
    pub fn snapshot(&mut self) -> HealthSnapshot {
        self.refresh_sys_if_due();

        let n = self.frames_ms.len();
        let (fps, avg, p95, p99, worst, over_pct) = if n == 0 {
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        } else {
            let mut sorted: Vec<f32> = self.frames_ms.iter().copied().collect();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let sum: f32 = sorted.iter().sum();
            let avg = sum / n as f32;
            let worst = *sorted.last().unwrap();
            let budget_ms = self.cfg.frame_budget.as_secs_f32() * 1e3;
            let over = sorted.iter().filter(|&&ms| ms > budget_ms).count();
            (
                1e3 * n as f32 / sum,
                avg,
                percentile(&sorted, 0.95),
                percentile(&sorted, 0.99),
                worst,
                100.0 * over as f32 / n as f32,
            )
        };

        HealthSnapshot {
            uptime_s: self.start.elapsed().as_secs_f64(),
            frames_total: self.frames_total,
            fps,
            frame_avg_ms: avg,
            frame_p95_ms: p95,
            frame_p99_ms: p99,
            frame_worst_ms: worst,
            over_budget_window_pct: over_pct,
            over_budget_total: self.over_budget_total,
            rss_mib: self.cached_rss_mib,
            cpu_pct: self.cached_cpu_pct,
        }
    }

    fn refresh_sys_if_due(&mut self) {
        let Some(pid) = self.pid else { return };
        let due = self
            .last_sys_refresh
            .is_none_or(|t| t.elapsed() >= self.cfg.sys_refresh_interval);
        if !due {
            return;
        }
        self.last_sys_refresh = Some(Instant::now());
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
        if let Some(proc_) = self.sys.process(pid) {
            self.cached_rss_mib = Some(proc_.memory() as f64 / (1024.0 * 1024.0));
            // First CPU sample after startup is meaningless (needs a delta);
            // sysinfo reports 0.0 there, which is fine for our purposes.
            self.cached_cpu_pct = Some(proc_.cpu_usage());
        }
    }
}

/// Nearest-rank percentile over an ascending-sorted slice.
fn percentile(sorted: &[f32], q: f32) -> f32 {
    debug_assert!(!sorted.is_empty());
    let rank = (q * sorted.len() as f32).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> HealthMonitor {
        HealthMonitor::new(HealthConfig::default())
    }

    #[test]
    fn stats_from_synthetic_frames() {
        let mut m = monitor();
        // 95 fast frames, 5 slow ones.
        for _ in 0..95 {
            m.on_frame(Duration::from_millis(10));
        }
        for _ in 0..5 {
            m.on_frame(Duration::from_millis(30));
        }
        let s = m.snapshot();
        assert_eq!(s.frames_total, 100);
        assert_eq!(s.over_budget_total, 5);
        assert!((s.over_budget_window_pct - 5.0).abs() < 1e-3);
        assert!(s.frame_avg_ms > 10.0 && s.frame_avg_ms < 12.0);
        assert_eq!(s.frame_worst_ms, 30.0);
        assert!(s.frame_p95_ms <= s.frame_p99_ms);
        assert!(s.frame_p99_ms <= s.frame_worst_ms);
        // 100 frames * ~11ms avg → ~90 fps equivalent.
        assert!(s.fps > 80.0 && s.fps < 100.0, "fps {}", s.fps);
    }

    #[test]
    fn window_rolls_but_totals_accumulate() {
        let mut m = HealthMonitor::new(HealthConfig {
            window: 10,
            ..Default::default()
        });
        for _ in 0..10 {
            m.on_frame(Duration::from_millis(20)); // all over budget
        }
        for _ in 0..10 {
            m.on_frame(Duration::from_millis(5)); // pushes slow frames out
        }
        let s = m.snapshot();
        assert_eq!(s.over_budget_window_pct, 0.0);
        assert_eq!(s.over_budget_total, 10);
        assert_eq!(s.frames_total, 20);
    }

    #[test]
    fn empty_monitor_reports_zeros() {
        let mut m = monitor();
        let s = m.snapshot();
        assert_eq!(s.fps, 0.0);
        assert_eq!(s.frames_total, 0);
    }

    #[test]
    fn rss_is_reported_on_this_platform() {
        let mut m = monitor();
        m.on_frame(Duration::from_millis(1));
        let s = m.snapshot();
        // A running test process definitely occupies memory.
        let rss = s.rss_mib.expect("rss should be available in tests");
        assert!(rss > 1.0, "rss {rss} MiB");
    }

    #[test]
    fn percentile_nearest_rank() {
        let v: Vec<f32> = (1..=100).map(|i| i as f32).collect();
        assert_eq!(percentile(&v, 0.95), 95.0);
        assert_eq!(percentile(&v, 0.99), 99.0);
        assert_eq!(percentile(&[42.0], 0.99), 42.0);
    }
}
