//! A sampling profile of the running server, with the `pprof` feature (off
//! by default). `pprof` is driven by SIGPROF, so it needs no perf-event
//! permissions.
//!
//! `WVS_PPROF=<prefix>` profiles `WVS_PPROF_SECS` seconds (60), starting
//! `WVS_PPROF_DELAY` seconds (0) after start-up, at `WVS_PPROF_HZ` (999),
//! then writes `<prefix>.svg`, a flame graph, and `<prefix>.txt`: the
//! samples by function and by line, by self time and charged to the
//! server's own innermost line, and by inclusive time. SIGPROF comes at the
//! kernel's tick, so a process gets about 100 samples a CPU-second,
//! whatever the rate asked. See docs/PERFORMANCE.md.

use ahash::{AHashMap, AHashSet};
use std::fmt::Write as _;
use std::time::Duration;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Start the profile `WVS_PPROF` asks for, on a thread of its own.
pub fn start_from_env() {
    let Ok(prefix) = std::env::var("WVS_PPROF") else {
        return;
    };
    let delay = Duration::from_secs(env_u64("WVS_PPROF_DELAY", 0));
    let secs = Duration::from_secs(env_u64("WVS_PPROF_SECS", 60));
    let hz = env_u64("WVS_PPROF_HZ", 999) as i32;
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let guard = match pprof::ProfilerGuardBuilder::default()
            .frequency(hz)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
        {
            Ok(g) => g,
            Err(e) => {
                tracing::error!("profile: {e}");
                return;
            }
        };
        tracing::info!("profile: {} s at {hz} Hz, into {prefix}.*", secs.as_secs());
        std::thread::sleep(secs);
        let report = match guard.report().build() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("profile: {e}");
                return;
            }
        };
        drop(guard);
        match std::fs::File::create(format!("{prefix}.svg")) {
            Ok(f) => {
                if let Err(e) = report.flamegraph(f) {
                    tracing::error!("profile: the flame graph: {e}");
                }
            }
            Err(e) => tracing::error!("profile: {prefix}.svg: {e}"),
        }
        let text = summary(&report);
        if let Err(e) = std::fs::write(format!("{prefix}.txt"), text) {
            tracing::error!("profile: {prefix}.txt: {e}");
        }
        tracing::info!("profile written to {prefix}.*");
    });
}

/// Where `path` is, short: from the crate (or library) directory on.
fn short(path: &str) -> &str {
    if let Some(rest) = path.strip_prefix(env!("CARGO_MANIFEST_DIR")) {
        return rest.trim_start_matches('/');
    }
    match path.rfind("/src/") {
        Some(i) => {
            let crate_start = path[..i].rfind('/').map_or(0, |j| j + 1);
            &path[crate_start..]
        }
        None => path,
    }
}

/// The samples by function and by line: by self time, by self time
/// charged to the innermost line of the server's own source, and by
/// inclusive time (each function once a stack).
fn summary(report: &pprof::Report) -> String {
    let mut own: AHashMap<String, u64> = AHashMap::new();
    let mut own_line: AHashMap<String, u64> = AHashMap::new();
    let mut ours: AHashMap<String, u64> = AHashMap::new();
    let mut inclusive: AHashMap<String, u64> = AHashMap::new();
    let mut ours_inclusive: AHashMap<String, u64> = AHashMap::new();
    let mut total = 0u64;
    let manifest = env!("CARGO_MANIFEST_DIR");
    for (frames, &count) in &report.data {
        let count = count as u64;
        total += count;
        // Innermost first.
        let symbols: Vec<&pprof::Symbol> = frames.frames.iter().flatten().collect();
        if let Some(s) = symbols.first() {
            *own.entry(s.name()).or_default() += count;
            let line = format!(
                "{}:{} {}",
                short(&s.filename()),
                s.lineno.unwrap_or(0),
                s.name()
            );
            *own_line.entry(line).or_default() += count;
        }
        if let Some(s) = symbols.iter().find(|s| s.filename().starts_with(manifest)) {
            let line = format!(
                "{}:{} {}",
                short(&s.filename()),
                s.lineno.unwrap_or(0),
                s.name()
            );
            *ours.entry(line).or_default() += count;
        }
        let mut seen = AHashSet::new();
        let mut seen_ours = AHashSet::new();
        for s in &symbols {
            let file = s.filename();
            let name = format!("{} ({})", s.name(), short(&file));
            if file.starts_with(manifest) && seen_ours.insert(name.clone()) {
                *ours_inclusive.entry(name.clone()).or_default() += count;
            }
            if seen.insert(name.clone()) {
                *inclusive.entry(name).or_default() += count;
            }
        }
    }
    let pct = |c: u64| 100.0 * c as f64 / total.max(1) as f64;
    let mut out = format!("{total} samples\n");
    for (title, map, k) in [
        ("self, by function", &own, 40),
        ("self, by line", &own_line, 40),
        ("self, charged to the server's innermost line", &ours, 60),
        ("inclusive, the server's functions", &ours_inclusive, 120),
        ("inclusive, everything", &inclusive, 60),
    ] {
        let _ = writeln!(out, "-- {title}");
        let mut v: Vec<_> = map.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (name, &c) in v.into_iter().take(k) {
            let _ = writeln!(out, "{:5.1}%  {name}", pct(c));
        }
    }
    out
}
