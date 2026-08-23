//! Linux `runsc` (gVisor) backend — **Phase 4**.
//!
//! The Sandbox Manager drives `runsc` directly (no Docker/Podman):
//!
//! 1. Locate `runsc` (env `DOCBUNKER_RUNSC_BIN` or `PATH`) and verify the
//!    version (`runsc --version`).
//! 2. Write an OCI bundle per session (`OciBundle`, hardened config: read-only
//!    rootfs, no capabilities, unprivileged user, no network, cgroup limits).
//! 3. `runsc run <container-id>` with `--bundle` and **piped stdio** — the
//!    worker becomes the container init and speaks the binary protocol over
//!    the pipes (the same contract as the subprocess backend).
//! 4. Host-side wall-clock timeouts on every operation; on timeout/crash the
//!    sandbox is killed and deleted.
//! 5. Guaranteed cleanup: `runsc delete`, bundle removal, state cleanup.
//!
//! All `runsc` invocations use `std::process::Command` with separated
//! arguments — never shell strings (threat model A21).
//!
//! Note: `runsc` must be able to create its sandbox (it requires elevated
//! privileges on the host); this backend assumes the deployment provides that.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::backend::SandboxBackend;
use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::process::{ProcessTransport, WorkerSession};
use crate::runsc_bundle::{parse_runsc_version, OciBundle};
use crate::session::{DocumentId, DocumentInput, SandboxKind, SandboxSession};
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

/// Runtime configuration for the `runsc` backend.
#[derive(Debug, Clone)]
pub struct RunscConfig {
    /// `runsc` binary (path or bare name resolved via PATH).
    pub runsc_bin: PathBuf,
    /// Rootfs directory containing `/bin/renderer-worker` (read-only).
    pub rootfs_dir: PathBuf,
    /// Base directory for per-session bundles and runsc state.
    pub tmp_base: PathBuf,
}

impl RunscConfig {
    pub fn new(
        runsc_bin: impl AsRef<Path>,
        rootfs_dir: impl AsRef<Path>,
        tmp_base: impl AsRef<Path>,
    ) -> Self {
        Self {
            runsc_bin: runsc_bin.as_ref().to_path_buf(),
            rootfs_dir: rootfs_dir.as_ref().to_path_buf(),
            tmp_base: tmp_base.as_ref().to_path_buf(),
        }
    }
}

struct RunscSession {
    config: SandboxConfig,
    _bundle_dir: tempfile::TempDir,
    container_id: String,
    worker: WorkerSession,
}

/// gVisor (`runsc`) sandbox backend for Linux.
pub struct RunscBackend {
    config: RunscConfig,
    state_dir: Option<tempfile::TempDir>,
    next_session_id: u64,
    next_document_id: u64,
    sessions: HashMap<u64, RunscSession>,
}

impl RunscBackend {
    pub fn new(config: RunscConfig) -> Self {
        Self {
            config,
            state_dir: None,
            next_session_id: 0,
            next_document_id: 0,
            sessions: HashMap::new(),
        }
    }

    /// Build a `runsc` command with the hardened global flags.
    fn runsc(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.config.runsc_bin);
        cmd.arg("--root")
            .arg(self.state_dir_path())
            .arg("--network=none")
            .arg("--log")
            .arg(self.state_dir_path().join("runsc.log"));
        for arg in args {
            cmd.arg(arg);
        }
        cmd
    }

    fn state_dir_path(&self) -> &Path {
        self.state_dir
            .as_ref()
            .expect("RunscBackend must be initialized")
            .path()
    }

    /// Best-effort teardown of a session's sandbox and bundle.
    fn destroy_sandbox(&self, session: &RunscSession) {
        // `runsc delete` stops and removes the container. Use --force so a
        // partially-started or already-stopped container is cleaned up too.
        cleanup_container(
            &self.config.runsc_bin,
            self.state_dir_path(),
            &session.container_id,
            session.config.shutdown_timeout,
        );
    }

    fn probe_runsc(&self) -> Result<Output, SandboxError> {
        let mut command = Command::new(&self.config.runsc_bin);
        command.arg("--version");
        let output = output_with_timeout(&mut command, Duration::from_secs(5));
        match output {
            Ok(Some(out)) if out.status.success() => Ok(out),
            Ok(None) => {
                tracing::error!("runsc --version timed out");
                Err(SandboxError::BackendUnsupported("runsc probe timed out"))
            }
            Ok(out) => {
                let out = out.expect("successful probe result checked above");
                tracing::error!(
                    "runsc --version failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                Err(SandboxError::BackendUnsupported("runsc not usable"))
            }
            Err(e) => {
                tracing::error!(%e, "runsc binary not found");
                Err(SandboxError::BackendUnsupported("runsc not found"))
            }
        }
    }
}

fn validate_tmp_base(path: &Path) -> Result<(), SandboxError> {
    std::fs::create_dir_all(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SandboxError::Internal(
            "runsc temporary base must be a real directory".into(),
        ));
    }
    Ok(())
}

fn private_tempdir(base: &Path, prefix: &str) -> Result<tempfile::TempDir, SandboxError> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix).rand_bytes(16);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    Ok(builder.tempdir_in(base)?)
}

fn output_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<Option<Output>> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout configured as piped");
    let stderr = child.stderr.take().expect("stderr configured as piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let (status, completed) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, true);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            break (child.wait()?, false);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Descendants may inherit the pipes, so never let diagnostic collection
    // defeat the command timeout after the direct child has been reaped.
    let drain_started = Instant::now();
    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
        && drain_started.elapsed() < Duration::from_millis(250)
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    let stdout = if stdout_reader.is_finished() {
        stdout_reader.join().unwrap_or_default()
    } else {
        tracing::warn!("runsc stdout reader thread blocked; detaching");
        Vec::new()
    };
    let stderr = if stderr_reader.is_finished() {
        stderr_reader.join().unwrap_or_default()
    } else {
        tracing::warn!("runsc stderr reader thread blocked; detaching");
        Vec::new()
    };
    Ok(completed.then_some(Output {
        status,
        stdout,
        stderr,
    }))
}

fn cleanup_container(runsc_bin: &Path, state_dir: &Path, container_id: &str, timeout: Duration) {
    let mut command = Command::new(runsc_bin);
    command
        .arg("--root")
        .arg(state_dir)
        .arg("--network=none")
        .arg("delete")
        .arg("--force")
        .arg(container_id);
    match output_with_timeout(&mut command, timeout) {
        Ok(Some(out)) if out.status.success() => {}
        Ok(Some(out)) => tracing::warn!(
            status = %out.status,
            "runsc delete failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Ok(None) => tracing::warn!("runsc delete timed out"),
        Err(e) => tracing::warn!(%e, "runsc delete could not run"),
    }
}

struct StartupGuard<'a> {
    runsc_bin: &'a Path,
    state_dir: &'a Path,
    container_id: &'a str,
    timeout: Duration,
    armed: bool,
}

impl Drop for StartupGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            cleanup_container(
                self.runsc_bin,
                self.state_dir,
                self.container_id,
                self.timeout,
            );
        }
    }
}

impl SandboxBackend for RunscBackend {
    fn initialize(&mut self) -> Result<(), SandboxError> {
        let out = self.probe_runsc()?;
        if let Ok(text) = String::from_utf8(out.stdout) {
            match parse_runsc_version(&text) {
                Some(v) => tracing::info!(version = %v, "runsc version"),
                None => tracing::warn!("unexpected runsc --version output: {}", text.trim()),
            }
        }

        // Rootfs must exist and contain the worker binary.
        if !self.config.rootfs_dir.join("bin/renderer-worker").is_file() {
            return Err(SandboxError::BackendUnsupported(
                "rootfs missing /bin/renderer-worker",
            ));
        }

        validate_tmp_base(&self.config.tmp_base)?;
        self.state_dir = Some(private_tempdir(
            &self.config.tmp_base,
            "docbunker-runsc-state-",
        )?);

        tracing::info!("RunscBackend initialized");
        Ok(())
    }

    fn start_session(&mut self, config: SandboxConfig) -> Result<SandboxSession, SandboxError> {
        config
            .validate()
            .map_err(|msg| SandboxError::Internal(msg.into()))?;

        self.next_session_id += 1;
        let id = self.next_session_id;
        let bundle_dir = private_tempdir(&self.config.tmp_base, "docbunker-bundle-")?;
        let container_id = bundle_dir
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SandboxError::Internal("invalid temporary bundle name".into()))?
            .to_owned();

        OciBundle::write(bundle_dir.path(), &self.config.rootfs_dir, &config)?;
        let mut startup = StartupGuard {
            runsc_bin: &self.config.runsc_bin,
            state_dir: self.state_dir_path(),
            container_id: &container_id,
            timeout: config.shutdown_timeout,
            armed: true,
        };

        let mut command = self.runsc(&["run", "--bundle"]);
        command.arg(bundle_dir.path()).arg(&container_id);
        let transport = ProcessTransport::spawn_command(&mut command, config.operation_timeout)?;
        startup.armed = false;
        drop(startup);

        self.sessions.insert(
            id,
            RunscSession {
                config,
                _bundle_dir: bundle_dir,
                container_id,
                worker: WorkerSession::new(transport),
            },
        );
        Ok(SandboxSession {
            id,
            kind: SandboxKind::Runsc,
        })
    }

    fn send_document(
        &mut self,
        session: &mut SandboxSession,
        document: DocumentInput,
    ) -> Result<DocumentId, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if document.data.len() > session_state.config.max_document_size {
            return Err(docbunker_renderer_api::RenderError::DocumentTooLarge.into());
        }

        self.next_document_id += 1;
        let id = DocumentId(self.next_document_id);
        session_state.worker.send_document(document.data, id.0)?;
        Ok(id)
    }

    fn get_document_info(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
    ) -> Result<DocumentInfo, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;
        session_state.worker.get_document_info(document_id.0)
    }

    fn render_page(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if options.target_width > session_state.config.max_page_width
            || options.target_height > session_state.config.max_page_height
        {
            return Err(docbunker_renderer_api::RenderError::ResourceLimitExceeded.into());
        }
        session_state
            .worker
            .render_page(document_id.0, page, options)
    }

    fn close_session(&mut self, session: SandboxSession) -> Result<(), SandboxError> {
        let Some(session_state) = self.sessions.remove(&session.id) else {
            return Err(SandboxError::InvalidSession);
        };

        let mut session_state = session_state;
        session_state.worker.close_all_documents();
        session_state.worker.shutdown();
        self.destroy_sandbox(&session_state);
        Ok(())
    }
}

impl Drop for RunscBackend {
    fn drop(&mut self) {
        let Some(state_dir) = self.state_dir.as_ref() else {
            return;
        };
        for (_, mut session) in self.sessions.drain() {
            session.worker.terminate();
            cleanup_container(
                &self.config.runsc_bin,
                state_dir.path(),
                &session.container_id,
                session.config.shutdown_timeout,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_directories_are_unique() {
        let base = tempfile::tempdir().unwrap();
        let first = private_tempdir(base.path(), "docbunker-test-").unwrap();
        let second = private_tempdir(base.path(), "docbunker-test-").unwrap();
        assert_ne!(first.path(), second.path());

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(first.path().metadata().unwrap().mode() & 0o777, 0o700);
            assert_eq!(second.path().metadata().unwrap().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn temporary_base_must_be_a_directory() {
        let base = tempfile::NamedTempFile::new().unwrap();
        assert!(validate_tmp_base(base.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn command_timeout_is_bounded() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 5");
        let started = Instant::now();
        let output = output_with_timeout(&mut command, Duration::from_millis(20)).unwrap();
        assert!(output.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

/// End-to-end test against a real `runsc` (Linux only).
///
/// Requires: `DOCBUNKER_RUNSC_BIN` (or runsc on PATH), a rootfs in
/// `DOCBUNKER_ROOTFS` containing `bin/renderer-worker`, and privileges for
/// runsc. Run with: `cargo test -p docbunker-sandbox runsc_end_to_end -- --ignored`
#[cfg(all(test, target_os = "linux"))]
#[test]
#[ignore = "requires runsc + a prepared rootfs on a Linux host"]
fn runsc_end_to_end() {
    use docbunker_renderer_api::DocumentFormat;

    let runsc_bin = std::env::var("DOCBUNKER_RUNSC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("runsc"));
    let rootfs = PathBuf::from(
        std::env::var("DOCBUNKER_ROOTFS").unwrap_or_else(|_| "sandbox/rootfs".into()),
    );
    let tmp_base = std::env::temp_dir().join(format!("docbunker-runsc-it-{}", std::process::id()));

    let config = crate::platforms::RunscConfig::new(runsc_bin, rootfs, tmp_base.clone());
    let mut backend = RunscBackend::new(config);
    backend
        .initialize()
        .expect("runsc + rootfs must be available");

    let session_config = SandboxConfig {
        operation_timeout: std::time::Duration::from_secs(20),
        ..SandboxConfig::default()
    };
    let mut session = backend
        .start_session(session_config)
        .expect("session starts");

    let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([5, 6, 7, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("fixture encodes");

    let id = backend
        .send_document(&mut session, DocumentInput::new(out.into_inner()))
        .expect("document enters the sandbox");
    let info = backend.get_document_info(&mut session, id).expect("info");
    assert_eq!(info.format, DocumentFormat::Png);

    let page = backend
        .render_page(
            &mut session,
            id,
            0,
            RenderOptions {
                target_width: 16,
                target_height: 12,
            },
        )
        .expect("page renders inside runsc");
    page.validate().expect("validated page");
    assert_eq!(page.bytes.len(), 16 * 12 * 4);

    backend.close_session(session).expect("session closes");
    std::fs::remove_dir_all(&tmp_base).ok();
}

/// Escape test against a real `runsc` (Linux only).
///
/// This is the adversarial counterpart of `runsc_end_to_end`: instead of a
/// document it runs `escape-worker` — a deliberately compromised renderer —
/// as container init inside the **unchanged** OCI hardening profile, then
/// asserts that every attempted break-out failed:
///
/// - reading the host marker file (no host mounts) → blocked
/// - writing anywhere in the rootfs (read-only) → blocked
/// - touching sensitive `/proc` paths (masked/read-only) → blocked
/// - capabilities/uid/no-new-privs from `/proc/self/status` → hardened
/// - host environment (only `PATH=/bin` is configured) → no leak
/// - connecting to a host TCP listener (`--network=none`) → blocked
/// - spawning a shell (rootfs has none) → blocked
/// - host paths visible in the mount table → none
/// - anything the worker writes in `/tmp` landing on the host → absent
/// - process/thread fork bomb (pids limit) → bounded
///
/// Requires: `DOCBUNKER_RUNSC_BIN` (or runsc on PATH), `DOCBUNKER_ROOTFS`
/// (a prepared rootfs) and `DOCBUNKER_ESCAPE_WORKER` (the statically linked
/// `escape-worker` binary, e.g. built with the musl target). Linux + root
/// privileges for runsc.
///
/// Run with:
/// `cargo test -p docbunker-sandbox runsc_escape_test -- --ignored`
#[cfg(all(test, target_os = "linux"))]
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::fs::symlink;

    let metadata = std::fs::symlink_metadata(src)?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(src)?;
        return symlink(target, dst);
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst)?;
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
struct RunscCleanupGuard {
    runsc_bin: PathBuf,
    state_dir: PathBuf,
    container_id: String,
    timeout: Duration,
    armed: bool,
}

#[cfg(all(test, target_os = "linux"))]
impl Drop for RunscCleanupGuard {
    fn drop(&mut self) {
        if self.armed {
            cleanup_container(
                &self.runsc_bin,
                &self.state_dir,
                &self.container_id,
                self.timeout,
            );
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[test]
#[ignore = "requires runsc + rootfs + escape-worker on a Linux host"]
fn runsc_escape_test() {
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    let runsc_bin = std::env::var("DOCBUNKER_RUNSC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("runsc"));
    let rootfs = PathBuf::from(
        std::env::var("DOCBUNKER_ROOTFS").unwrap_or_else(|_| "sandbox/rootfs".into()),
    );
    let escape_worker = PathBuf::from(
        std::env::var("DOCBUNKER_ESCAPE_WORKER")
            .expect("DOCBUNKER_ESCAPE_WORKER must point at the escape-worker binary"),
    );
    assert!(
        escape_worker.is_file(),
        "escape-worker binary not found at {}",
        escape_worker.display()
    );

    // Host-side fixtures the sandbox must not reach.
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let marker = workspace.path().join("host-top-secret.txt");
    const HOST_SECRET: &[u8] = b"TOP-SECRET-HOST-DATA";
    std::fs::write(&marker, HOST_SECRET).expect("write host marker");
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).expect("0600 marker");
    let sentinel = format!(
        "docbunker-escape-sentinel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind host listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let port = listener.local_addr().expect("listener address").port();

    // Private rootfs copy whose /bin/renderer-worker IS the adversary; the
    // real (shared, read-only) rootfs is never touched.
    let rootfs_run = workspace.path().join("rootfs");
    copy_tree(&rootfs, &rootfs_run).expect("copy rootfs for escape run");
    let worker_path = rootfs_run.join("bin/renderer-worker");
    std::fs::copy(&escape_worker, &worker_path).expect("install escape worker");
    std::fs::set_permissions(&worker_path, std::fs::Permissions::from_mode(0o755))
        .expect("executable escape worker");

    let bundle = workspace.path().join("bundle");
    let session_config = SandboxConfig {
        operation_timeout: Duration::from_secs(120),
        shutdown_timeout: Duration::from_secs(20),
        memory_limit_bytes: Some(256 * 1024 * 1024),
        cpu_limit_millicpus: Some(500),
        max_processes: Some(64),
        ..SandboxConfig::default()
    };
    OciBundle::write_with_args(
        &bundle,
        &rootfs_run,
        &session_config,
        &[
            "/bin/renderer-worker".to_string(),
            marker.to_string_lossy().into_owned(),
            sentinel.clone(),
            port.to_string(),
        ],
    )
    .expect("write hardened bundle");

    let state_dir = workspace.path().join("runsc-state");
    std::fs::create_dir_all(&state_dir).expect("runsc state dir");
    let container_id = format!("docbunker-escape-{}", std::process::id());
    let guard = RunscCleanupGuard {
        runsc_bin: runsc_bin.clone(),
        state_dir: state_dir.clone(),
        container_id: container_id.clone(),
        timeout: session_config.shutdown_timeout,
        armed: true,
    };

    let mut command = Command::new(&runsc_bin);
    command
        .arg("--root")
        .arg(&state_dir)
        .arg("--network=none")
        .arg("--log")
        .arg(state_dir.join("runsc.log"))
        .arg("run")
        .arg("--bundle")
        .arg(&bundle)
        .arg(&container_id);
    let output =
        output_with_timeout(&mut command, Duration::from_secs(180)).expect("runsc must run");
    let Some(output) = output else {
        panic!("escape worker did not finish within 180s (sandbox hung)");
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse the adversary's report.
    let mut results: Vec<(String, bool, String)> = Vec::new();
    let mut report_done = false;
    for line in stdout.lines() {
        if line.contains("REPORT_DONE") {
            report_done = true;
        }
        if let Some(rest) = line.strip_prefix("ESCAPE|") {
            let mut parts = rest.splitn(4, '|');
            let check = parts.next().unwrap_or_default();
            let status = parts.next().unwrap_or_default();
            let detail = parts.next().unwrap_or_default();
            results.push((check.to_string(), status == "PASS", detail.to_string()));
        }
    }

    const EXPECTED_CHECKS: [&str; 10] = [
        "host-marker-unreadable",
        "rootfs-read-only",
        "proc-read-only",
        "status-hardened",
        "env-clean",
        "network-blocked",
        "exec-blocked",
        "mounts-host-free",
        "sandbox-tmp-private",
        "process-limit",
    ];
    let summarize = || {
        std::iter::once(format!("REPORT_DONE seen: {report_done}"))
            .chain(
                results
                    .iter()
                    .map(|(c, ok, d)| format!("{c}: {} ({d})", if *ok { "PASS" } else { "FAIL" })),
            )
            .collect::<Vec<_>>()
            .join("\n")
    };
    let report = summarize();

    assert!(
        output.status.success(),
        "runsc run failed\nstderr: {stderr}\nstdout: {stdout}"
    );
    assert!(
        report_done,
        "escape worker did not complete its report\n{report}\nstderr: {stderr}"
    );
    for expected in EXPECTED_CHECKS {
        let Some((_, ok, detail)) = results.iter().find(|(c, _, _)| c == expected) else {
            panic!("missing escape check {expected}\n{report}\nstderr: {stderr}");
        };
        assert!(
            *ok,
            "escape check {expected} FAILED: {detail}\n{report}\nstderr: {stderr}"
        );
    }

    // Host-side proof: the sandbox could neither read nor egress to the host.
    assert_eq!(
        std::fs::read(&marker).expect("host marker readable by test"),
        HOST_SECRET,
        "host marker content changed (sandbox wrote through?)",
    );
    for leaked in [
        std::env::temp_dir().join(&sentinel),
        workspace.path().join(&sentinel),
    ] {
        assert!(
            !leaked.exists(),
            "sandbox file leaked to host filesystem: {}",
            leaked.display()
        );
    }
    match listener.accept() {
        Ok((_, _)) => panic!("host listener accepted a connection from inside the sandbox"),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => panic!("listener error: {e}"),
    }
}
