//! Cost measurements of a process, read from /proc (Linux only): of this very one, or of another
//! one on the same machine, such as the language model.

use std::fs;

/// CPU seconds this process has used since it started.
pub fn cpu_seconds() -> f64 {
    cpu_seconds_of("self")
}

/// Memory this process uses now and the most it has ever used, in MB.
pub fn memory_mb() -> (f64, f64) {
    memory_mb_of("self")
}

/// CPU seconds the process `pid` has used since it started, in all its threads.
pub fn cpu_seconds_of(pid: &str) -> f64 {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0.0;
    };
    let Some((_, tail)) = text.rsplit_once(')') else {
        return 0.0;
    };
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let ticks = |i: usize| fields.get(i).and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0);
    (ticks(11) + ticks(12)) / 100.0
}

/// Memory the process `pid` uses now and the most it has ever used, in MB.
pub fn memory_mb_of(pid: &str) -> (f64, f64) {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse::<f64>().ok())
            .map_or(0.0, |kb| kb * 1024.0 / 1e6)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// The number of the first process on this machine with that name, if there is one.
pub fn process_named(name: &str) -> Option<String> {
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let pid = entry.file_name().to_string_lossy().into_owned();
        if pid.bytes().all(|b| b.is_ascii_digit()) && fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| comm.trim() == name) {
            return Some(pid);
        }
    }
    None
}
