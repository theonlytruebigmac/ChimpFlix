//! Per-process resource limits applied to long-running ffmpeg session
//! subprocesses. Defends against the most common transcoder DoS vector:
//! a single pathological media file triggering an unbounded ffmpeg
//! allocation that OOMs the whole host.
//!
//! Implementation strategy: `setrlimit(RLIMIT_AS, ...)` via the
//! `pre_exec` hook on `std::os::unix::process::CommandExt`. The hook
//! runs in the forked child between `fork()` and `execve()`, so the
//! limit is applied before ffmpeg's process image starts allocating.
//! `RLIMIT_AS` caps total virtual address space rather than RSS — that
//! catches the runaway-`mmap` pattern that backs nearly every ffmpeg
//! OOM, without misfiring on legitimate transient peaks the way an RSS
//! limit can.
//!
//! Non-Linux platforms are a no-op for now. Windows would need a
//! JobObject; macOS supports `setrlimit` similarly but the cap is
//! advisory and the test surface is smaller, so we leave it as
//! best-effort.
//!
//! See BLOCK #3 in `docs/PUBLIC_RELEASE_HARDENING.md`.

/// Apply per-session resource limits to `cmd`. Caller supplies the
/// virtual-memory cap in mebibytes; 0 disables the cap entirely.
/// No-op on non-Unix platforms.
#[cfg(unix)]
pub fn apply_session_limits(cmd: &mut tokio::process::Command, mem_mb: u64) {
    if mem_mb == 0 {
        return;
    }
    let bytes = (mem_mb as libc::rlim_t).saturating_mul(1024 * 1024);
    // SAFETY: `setrlimit` is async-signal-safe and the closure runs in
    // the forked child before `execve`, so we don't touch heap state
    // or take locks. Errors from setrlimit are turned into io::Error
    // so the parent sees a clean spawn failure rather than a child
    // booted under whatever limit happened to be inherited.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().pre_exec(move || {
            let rlim = libc::rlimit {
                rlim_cur: bytes,
                rlim_max: bytes,
            };
            if libc::setrlimit(libc::RLIMIT_AS, &rlim) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Windows / non-Unix stub. Intentionally inert until JobObjects (or
/// equivalent) get wired up. The transcoder remains usable on those
/// platforms; the per-session DoS surface is the operator's problem to
/// mitigate at the container layer.
#[cfg(not(unix))]
pub fn apply_session_limits(_cmd: &mut tokio::process::Command, _mem_mb: u64) {}
