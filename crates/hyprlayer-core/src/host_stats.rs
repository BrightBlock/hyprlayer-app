use hyprlayer_models::HostStats;
use sysinfo::{MemoryRefreshKind, System};

pub fn host_stats() -> Result<HostStats, sysinfo::Error> {
    let mut system = System::new()?;
    system.refresh_memory_specifics(MemoryRefreshKind::everything());
    let load = System::load_average()
        .ok()
        .map(|l| [l.one, l.five, l.fifteen]);
    Ok(HostStats {
        load,
        cpus: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
        memory_used: system.used_memory() as f64,
        memory_total: system.total_memory() as f64,
        swap_used: system.used_swap() as f64,
        swap_total: system.total_swap() as f64,
    })
}
