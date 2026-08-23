//! Shared helpers for integration tests (compiled per test binary, so
//! `CARGO_BIN_EXE_*` is available at compile time).

#![allow(dead_code)]

use std::process::{Child, Command, Stdio};

/// Spawn the `fake_worker` with the given misbehavior mode.
pub fn spawn_fake_worker(
    mode: &str,
) -> (Child, std::process::ChildStdin, std::process::ChildStdout) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake_worker"))
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn fake_worker");
    let stdin = child.stdin.take().expect("fake_worker stdin");
    let stdout = child.stdout.take().expect("fake_worker stdout");
    (child, stdin, stdout)
}

/// Kill the child if it is still running (best-effort cleanup).
pub fn kill_quietly(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
