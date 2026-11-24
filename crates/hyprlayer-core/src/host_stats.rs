use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hyprlayer_models::HostStats;
use sysinfo::{MemoryRefreshKind, System};

/// The Lead panel polls on the same beat, so 40 samples cover two minutes.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(3);
pub const HISTORY_LEN: usize = 40;
/// Usage measured over a shorter window is mostly noise, and several clients polling
/// at once would otherwise shrink each other's window.
const CPU_FLOOR: Duration = Duration::from_secs(1);

/// One `System` for the server's life: CPU usage is the delta between two refreshes,
/// so a fresh `System` per call can never report it.
pub struct HostSampler {
    inner: Mutex<Inner>,
}

struct Inner {
    system: System,
    cpu: CpuWindow,
}

impl HostSampler {
    pub fn new() -> Result<Self, sysinfo::Error> {
        Ok(Self {
            inner: Mutex::new(Inner {
                system: System::new()?,
                cpu: CpuWindow::default(),
            }),
        })
    }

    /// The background beat: refreshes CPU usage and appends it to the history.
    pub fn tick(&self) {
        let mut inner = self.lock();
        let Inner { system, cpu } = &mut *inner;
        cpu.tick(Instant::now(), || refresh_cpu(system));
    }

    pub fn stats(&self) -> HostStats {
        let mut inner = self.lock();
        let Inner { system, cpu } = &mut *inner;
        let cpu_percent = cpu.read(Instant::now(), || refresh_cpu(system));
        system.refresh_memory_specifics(MemoryRefreshKind::everything());
        let load = System::load_average()
            .ok()
            .map(|l| [l.one, l.five, l.fifteen]);
        let cpus = match system.cpus().len() {
            0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
            n => n,
        } as u32;
        HostStats {
            load,
            cpus,
            memory_used: system.used_memory() as f64,
            memory_total: system.total_memory() as f64,
            swap_used: system.used_swap() as f64,
            swap_total: system.total_swap() as f64,
            cpu_percent,
            cpu_history: Some(cpu.history()),
            memory_free: memory_free(system),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn refresh_cpu(system: &mut System) -> f32 {
    system.refresh_cpu_usage();
    system.global_cpu_usage()
}

/// macOS counts compressed and purgeable pages so that used + free leaves a remainder
/// that is not file cache, so the cache segment is not drawn there.
#[cfg(target_os = "macos")]
fn memory_free(_: &System) -> Option<f64> {
    None
}

#[cfg(not(target_os = "macos"))]
fn memory_free(system: &System) -> Option<f64> {
    Some(system.free_memory() as f64)
}

#[derive(Default)]
struct CpuWindow {
    refreshed: Option<Instant>,
    percent: Option<f32>,
    history: VecDeque<f32>,
}

impl CpuWindow {
    /// The first refresh only sets the baseline; usage exists from the second.
    fn read(&mut self, now: Instant, refresh: impl FnOnce() -> f32) -> Option<f32> {
        match self.refreshed {
            Some(at) if now.duration_since(at) < CPU_FLOOR => {}
            previous => {
                let usage = refresh();
                if previous.is_some() {
                    self.percent = Some(usage.clamp(0.0, 100.0));
                }
                self.refreshed = Some(now);
            }
        }
        self.percent
    }

    fn tick(&mut self, now: Instant, refresh: impl FnOnce() -> f32) {
        if let Some(percent) = self.read(now, refresh) {
            if self.history.len() == HISTORY_LEN {
                self.history.pop_front();
            }
            self.history.push_back(percent);
        }
    }

    fn history(&self) -> Vec<f32> {
        self.history.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_refresh_is_only_a_baseline() {
        let mut cpu = CpuWindow::default();
        let t0 = Instant::now();
        assert_eq!(cpu.read(t0, || 0.0), None);
        assert_eq!(cpu.read(t0 + Duration::from_secs(3), || 42.0), Some(42.0));
    }

    #[test]
    fn reads_inside_the_floor_reuse_the_last_value() {
        let mut cpu = CpuWindow::default();
        let t0 = Instant::now();
        cpu.read(t0, || 0.0);
        cpu.read(t0 + Duration::from_secs(2), || 30.0);
        let reread = cpu.read(t0 + Duration::from_millis(2_500), || {
            panic!("refreshed inside the floor")
        });
        assert_eq!(reread, Some(30.0));
        assert_eq!(cpu.read(t0 + Duration::from_secs(3), || 55.0), Some(55.0));
    }

    #[test]
    fn usage_is_clamped_to_a_percent() {
        let mut cpu = CpuWindow::default();
        let t0 = Instant::now();
        cpu.read(t0, || 0.0);
        assert_eq!(cpu.read(t0 + CPU_FLOOR, || 100.4), Some(100.0));
    }

    #[test]
    fn ticks_keep_the_last_two_minutes() {
        let mut cpu = CpuWindow::default();
        let t0 = Instant::now();
        for i in 0..=(HISTORY_LEN as u32 + 5) {
            cpu.tick(t0 + SAMPLE_EVERY * i, || i as f32);
        }
        let history = cpu.history();
        assert_eq!(history.len(), HISTORY_LEN);
        assert_eq!(history.first(), Some(&6.0));
        assert_eq!(history.last(), Some(&(HISTORY_LEN as f32 + 5.0)));
    }

    #[test]
    fn a_baseline_tick_adds_no_sample() {
        let mut cpu = CpuWindow::default();
        cpu.tick(Instant::now(), || 80.0);
        assert!(cpu.history().is_empty());
    }

    #[test]
    fn reports_cpu_from_the_second_read_and_leaves_room_for_cache() {
        let sampler = HostSampler::new().expect("sysinfo supports this host");
        assert_eq!(sampler.stats().cpu_percent, None);
        std::thread::sleep(CPU_FLOOR);
        sampler.tick();
        let stats = sampler.stats();
        assert!(stats.cpu_percent.is_some());
        assert_eq!(stats.cpu_history.map(|h| h.len()), Some(1));
        assert!(stats.cpus >= 1);
        if let Some(free) = stats.memory_free {
            assert!(free <= stats.memory_total - stats.memory_used);
        }
    }
}
