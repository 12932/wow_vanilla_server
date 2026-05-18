//! OS-level network packet counter sampling.
//!
//! Reads `/proc/net/dev` on Linux and returns aggregate RX/TX packet
//! counts across all non-loopback interfaces. Companion to the
//! application-level `wow_messages_per_second` counter — the OS view
//! is closer to what NIC + kernel actually shovel onto the wire, while
//! the WoW counter measures protocol-level messages BEFORE the writer
//! task batches up to 64 into a single TCP write.
//!
//! Expect the two metrics to diverge by 30-50× under broadcast-heavy
//! load (one TCP segment carries dozens of small movement opcodes).
//!
//! Non-Linux platforms return `None` from [`sample`]; the caller is
//! expected to skip the OS plot in that case.

/// Lifetime cumulative packet counters from the kernel's view.
/// Both fields are aggregated over every non-loopback interface.
#[derive(Debug, Clone, Copy)]
pub struct NetStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// Snapshot the current cumulative packet counters. Returns `None` on
/// non-Linux hosts or if `/proc/net/dev` is unreadable / malformed.
#[cfg(target_os = "linux")]
pub fn sample() -> Option<NetStats> {
    // `/proc/net/dev` is a virtual file — reads are cheap (one syscall,
    // no disk I/O). Per-line format (from kernel docs):
    //
    //   "  iface: rx_bytes rx_packets rx_errs rx_drop rx_fifo rx_frame
    //                rx_compressed rx_multicast tx_bytes tx_packets
    //                tx_errs tx_drop tx_fifo tx_colls tx_carrier
    //                tx_compressed"
    //
    // First two lines are the column header.
    let contents = std::fs::read_to_string("/proc/net/dev").ok()?;
    let mut rx = 0_u64;
    let mut tx = 0_u64;
    for line in contents.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        // Skip the loopback interface — its packets don't correspond
        // to wire traffic and would inflate the metric for local
        // testing.
        if name.trim() == "lo" {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // Need at least the rx_packets (index 1) and tx_packets (index 9).
        if fields.len() < 10 {
            continue;
        }
        rx += fields[1].parse::<u64>().unwrap_or(0);
        tx += fields[9].parse::<u64>().unwrap_or(0);
    }
    Some(NetStats {
        rx_packets: rx,
        tx_packets: tx,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn sample() -> Option<NetStats> {
    None
}
