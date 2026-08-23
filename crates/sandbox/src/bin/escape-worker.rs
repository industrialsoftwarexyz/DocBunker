//! Adversarial sandbox init for escape tests (test-only, never shipped).
//!
//! `escape-worker` plays the role of a **compromised renderer**: given code
//! execution inside the sandbox, it deliberately attempts to break the
//! isolation contract — read host files, write the rootfs, reach the
//! network, spawn a shell, read host environment, and exceed the process
//! limit. The host-side test (`runsc_escape_test`) drops this binary into a
//! private rootfs copy so the *unchanged* OCI hardening profile is what gets
//! attacked.
//!
//! Report protocol (stdout, one line per check, flushed immediately):
//! `ESCAPE:<check>:PASS` or `ESCAPE:<check>:FAIL:<detail>`, plus a final
//! `REPORT_DONE` marker. Exit code is always 0; the host asserts on lines.
//!
//! Arguments:
//!   1. host marker file path  — must NOT be readable from inside the sandbox
//!   2. /tmp sentinel name     — the sandbox must never leak it to the host
//!   3. host TCP port          — must NOT be connectable from inside
//!
//! The binary is intentionally std-only and builds on every host; it only
//! ever runs on Linux inside `runsc`/gVisor.

use std::env;
use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

fn report(check: &str, ok: bool, detail: &str) {
    let status = if ok { "PASS" } else { "FAIL" };
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "ESCAPE|{check}|{status}|{detail}");
    let _ = lock.flush();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let marker = args.get(1).map(String::as_str).unwrap_or("");
    let sentinel = args.get(2).map(String::as_str).unwrap_or("");
    let port: u16 = args.get(3).and_then(|p| p.parse().ok()).unwrap_or(0);

    check_host_marker_unreadable(marker);
    check_rootfs_read_only();
    check_proc_read_only();
    check_status_hardened();
    check_env_clean();
    check_network_blocked(port);
    check_exec_blocked();
    check_mounts_do_not_expose_host(marker, sentinel);
    check_sandbox_tmp_does_not_leak(sentinel);
    check_process_limit();

    report("report-done", true, "all checks attempted");
}

fn check_host_marker_unreadable(marker: &str) {
    if marker.is_empty() {
        report("host-marker-unreadable", false, "no marker argv supplied");
        return;
    }
    match fs::read(marker) {
        Ok(bytes) => report(
            "host-marker-unreadable",
            false,
            &format!("host file readable ({} bytes)", bytes.len()),
        ),
        Err(e) => report("host-marker-unreadable", true, &e.to_string()),
    }
}

fn check_rootfs_read_only() {
    let attempts = [
        "/esc-marker",
        "/bin/esc-marker",
        "/etc/esc-marker",
        "/usr/esc-marker",
    ];
    let mut wrote_any = false;
    for path in attempts {
        if fs::write(path, b"pwned").is_ok() {
            wrote_any = true;
        }
    }
    if wrote_any {
        report(
            "rootfs-read-only",
            false,
            "rootfs accepted a write (rootfs is not read-only)",
        );
    } else {
        report("rootfs-read-only", true, "every write rejected");
    }
}

fn check_proc_read_only() {
    let mut blocked = 0usize;
    for path in [
        "/proc/sysrq-trigger",
        "/proc/sys/kernel/hostname",
        "/proc/kcore",
        "/proc/self/mem",
    ] {
        let write_blocked = fs::write(path, b"x").is_err();
        let read_blocked = fs::read(path).is_err();
        if write_blocked && read_blocked {
            blocked += 1;
        }
    }
    if blocked == 4 {
        report("proc-read-only", true, "all proc sensitive paths blocked");
    } else {
        report(
            "proc-read-only",
            false,
            &format!("{}/4 sensitive /proc paths were accessible", 4 - blocked),
        );
    }
}

fn check_env_clean() {
    let mut vars: Vec<(String, String)> = env::vars().collect();
    vars.sort();
    if vars == [("PATH".to_string(), "/bin".to_string())] {
        report("env-clean", true, "only PATH=/bin present");
    } else {
        report(
            "env-clean",
            false,
            &format!("unexpected environment: {vars:?}"),
        );
    }
}

fn check_network_blocked(port: u16) {
    if port == 0 {
        report("network-blocked", false, "no port argv supplied");
        return;
    }
    match TcpStream::connect(("127.0.0.1", port)) {
        Ok(_) => report("network-blocked", false, "connected to host listener"),
        Err(e) => report("network-blocked", true, &e.to_string()),
    }
}

fn check_exec_blocked() {
    let candidates = [
        "/bin/sh",
        "/bin/bash",
        "/bin/busybox",
        "/bin/ls",
        "/usr/bin/env",
        "/usr/bin/python3",
    ];
    for binary in candidates {
        match Command::new(binary).output() {
            Ok(output) => {
                report(
                    "exec-blocked",
                    false,
                    &format!("{binary} ran (exit {})", output.status),
                );
                return;
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound
                    && e.kind() != std::io::ErrorKind::PermissionDenied
                {
                    report(
                        "exec-blocked",
                        false,
                        &format!("{binary}: unexpected spawn error {e}"),
                    );
                    return;
                }
            }
        }
    }
    report("exec-blocked", true, "no shell or tools present");
}

fn check_status_hardened() {
    let status = match fs::read_to_string("/proc/self/status") {
        Ok(text) => text,
        Err(e) => {
            report(
                "status-hardened",
                false,
                &format!("cannot read /proc/self/status: {e}"),
            );
            return;
        }
    };
    let mut uid_ok = false;
    let mut capeff_ok = false;
    let mut capbnd_ok = false;
    let mut nnp_ok = false;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let ids: Vec<&str> = rest.split_whitespace().collect();
            uid_ok = ids == ["65534", "65534", "65534", "65534"];
        } else if let Some(rest) = line.strip_prefix("CapEff:") {
            let value = rest.trim();
            capeff_ok = value == "0000000000000000";
        } else if let Some(rest) = line.strip_prefix("CapBnd:") {
            let value = rest.trim();
            capbnd_ok = value == "0000000000000000";
        } else if let Some(rest) = line.strip_prefix("NoNewPrivs:") {
            nnp_ok = rest.trim() == "1";
        }
    }
    let hardened = uid_ok && capeff_ok && capbnd_ok && nnp_ok;
    report(
        "status-hardened",
        hardened,
        &format!("uid={uid_ok} capeff={capeff_ok} capbnd={capbnd_ok} no_new_privs={nnp_ok}"),
    );
}

fn check_mounts_do_not_expose_host(marker: &str, sentinel: &str) {
    let text = match fs::read_to_string("/proc/self/mountinfo") {
        Ok(text) => text,
        Err(e) => {
            report(
                "mounts-host-free",
                false,
                &format!("cannot read mountinfo: {e}"),
            );
            return;
        }
    };
    let mut leaked = Vec::new();
    if !marker.is_empty() && text.contains(marker) {
        leaked.push("marker path");
    }
    if !sentinel.is_empty() && text.contains(sentinel) {
        leaked.push("sentinel name");
    }
    if leaked.is_empty() {
        report(
            "mounts-host-free",
            true,
            "no host path visible in mount table",
        );
    } else {
        report(
            "mounts-host-free",
            false,
            &format!("mount table references host: {}", leaked.join(", ")),
        );
    }
}

fn check_sandbox_tmp_does_not_leak(sentinel: &str) {
    if sentinel.is_empty() {
        report("sandbox-tmp-private", false, "no sentinel argv supplied");
        return;
    }
    let path = format!("/tmp/{sentinel}");
    match fs::write(&path, b"inside-sandbox") {
        Ok(()) => report(
            "sandbox-tmp-private",
            true,
            &format!("wrote sandbox /tmp/{sentinel}; host must not see it"),
        ),
        Err(e) => report(
            "sandbox-tmp-private",
            true,
            &format!("write rejected ({e}); impossible to leak either way"),
        ),
    }
}

fn check_process_limit() {
    const ATTEMPTS: usize = 256;
    let mut created = 0usize;
    let mut last_error = String::new();
    let mut handles = Vec::new();
    for _ in 0..ATTEMPTS {
        let handle = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| std::thread::sleep(Duration::from_millis(50)));
        match handle {
            Ok(handle) => {
                created += 1;
                handles.push(handle);
            }
            Err(e) => {
                last_error = e.to_string();
                break;
            }
        }
    }
    for handle in handles {
        let _ = handle.join();
    }
    // pids.max = 64 counts the worker itself, so a 64-heavy limit allows at
    // most 63 sibling tasks; any successful runaway would create all 256.
    if created < 64 {
        report(
            "process-limit",
            true,
            &format!("fork stopped after {created} tasks ({last_error})"),
        );
    } else {
        report(
            "process-limit",
            false,
            &format!("created {created} tasks without hitting a limit"),
        );
    }
}
