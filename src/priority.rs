//! The server's CPU priority (`[server] nice`): set at start-up for every
//! thread it has, and so for every thread it makes after (each takes its
//! maker's). Where the kernel groups a session's processes for the
//! scheduler (`sched_autogroup`), a process's niceness weighs it only
//! against the rest of its own group: the log says so. The group's own
//! niceness (`/proc/<pid>/autogroup`) is the whole session's, a shell's and
//! whatever else runs in it, and isn't the server's to change.

/// What was done, as the log says it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Threads set to the niceness.
    pub threads: usize,
    /// Why it was refused, if it was.
    pub refused: Option<String>,
    /// The kernel groups the session's processes (`sched_autogroup`).
    pub autogroup: bool,
}

/// Set the server's niceness to `nice` (-20, the most CPU, to 19), logging
/// what was done; nothing for 0.
pub fn apply(nice: i32) -> Applied {
    if nice == 0 {
        return Applied::default();
    }
    let applied = set(nice);
    let grouped = if applied.autogroup {
        " With sched_autogroup on, it weighs the server only against the other processes of \
         its session; against other sessions' it is their groups' niceness that counts \
         (`/proc/<pid>/autogroup`, the session's: a service of its own has one to itself)."
    } else {
        ""
    };
    match &applied.refused {
        None => tracing::info!(
            "nice {nice}: the server's {} threads.{grouped}",
            applied.threads
        ),
        Some(e) => tracing::warn!(
            "nice {nice} refused ({e}): the server runs at its niceness as started. {}",
            if nice < 0 {
                HOW
            } else {
                "A process may always raise its own niceness: this is unexpected."
            }
        ),
    }
    applied
}

/// How a niceness under 0 is granted.
const HOW: &str = "An unprivileged process can't go below 0: grant it with \
     `setcap cap_sys_nice+ep` on the binary, a `nice` limit in /etc/security/limits.conf \
     (or `ulimit -e`), or systemd's `Nice=` for its service";

#[cfg(target_os = "linux")]
fn set(nice: i32) -> Applied {
    let mut applied = Applied::default();
    // Each thread its own niceness on Linux: every one the process has now
    // (the runtime's workers among them).
    let tids: Vec<libc::id_t> = std::fs::read_dir("/proc/self/task")
        .map(|d| {
            d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    for tid in tids {
        // SAFETY: setpriority reads its arguments; `tid` is a thread of
        // this process.
        let r = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid, nice) };
        if r == 0 {
            applied.threads += 1;
        } else if applied.refused.is_none() {
            applied.refused = Some(std::io::Error::last_os_error().to_string());
        }
    }
    applied.autogroup = std::fs::read_to_string("/proc/sys/kernel/sched_autogroup_enabled")
        .is_ok_and(|s| s.trim() == "1");
    applied
}

#[cfg(not(target_os = "linux"))]
fn set(nice: i32) -> Applied {
    #[cfg(unix)]
    {
        // SAFETY: setpriority reads its arguments.
        let r = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) };
        return Applied {
            threads: usize::from(r == 0),
            refused: (r != 0).then(|| std::io::Error::last_os_error().to_string()),
            autogroup: false,
        };
    }
    #[cfg(not(unix))]
    Applied {
        threads: 0,
        refused: Some("not on this system".into()),
        autogroup: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_does_nothing() {
        assert_eq!(apply(0), Applied::default());
    }

    /// A process may always raise its own niceness: every thread set (the
    /// test's process alone: nextest's own).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_higher_niceness_is_set_on_every_thread() {
        let a = apply(5);
        assert!(a.refused.is_none(), "{a:?}");
        assert!(a.threads >= 1);
        // SAFETY: getpriority reads its arguments.
        let now = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        assert!(now >= 5, "{now}");
    }
}
