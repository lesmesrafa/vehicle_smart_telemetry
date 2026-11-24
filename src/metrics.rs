//! Cost measurements of this very process, read from /proc (Linux only).

use std::fs;

/// CPU seconds this process has used since it started.
pub fn cpu_seconds() -> f64 {
    let Ok(text) = fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    let Some((_, tail)) = text.rsplit_once(')') else {
        return 0.0;
    };
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let ticks = |i: usize| fields.get(i).and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0);
    (ticks(11) + ticks(12)) / 100.0
}
/// Memory the process uses now and the most it has ever used, in MB.
pub fn memory_mb() -> (f64, f64) {
    let text = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse::<f64>().ok())
            .map_or(0.0, |kb| kb * 1024.0 / 1e6)
    };
    (field("VmRSS:"), field("VmHWM:"))
}
