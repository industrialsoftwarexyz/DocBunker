//! A transport to the `renderer-worker` as a **child process**.
//!
//! `ProcessTransport` speaks the binary protocol over the child's
//! stdin/stdout pipes. It enforces:
//!
//! - a wall-clock timeout on every request/response exchange;
//! - strict response validation (same rules as
//!   [`crate::transport::InProcessTransport`]);
//! - cleanup: the child is killed and reaped on timeout, on error and on drop.
//!
//! **Security**: a bare worker child provides **no isolation**. This transport
//! is the base the `runsc` backend (Phase 4) builds on — never use a bare
//! `SubprocessBackend` with untrusted documents.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::SandboxError;
use crate::transport::SandboxTransport;
use docbunker_protocol::message::{
    CloseDocumentRequest, ErrorCode, GetDocumentInfoRequest, HelloRequest, OpenDocumentRequest,
    RenderPageRequest, Request, Response,
};
use docbunker_protocol::validate::{
    document_info_from_wire, rendered_page_from_wire, validate_error_response, NegotiatedLimits,
};
use docbunker_protocol::{
    decode_response, encode_request, FrameReader, FrameWriter, MAX_IPC_MESSAGE, PROTOCOL_VERSION,
};
use docbunker_renderer_api::limits as domain_limits;
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

type FrameResult = Result<Option<Vec<u8>>, String>;
type WriteResult = Result<(), String>;
type BoxedWriter = Box<dyn Write + Send>;
type BoxedReader = Box<dyn Read + Send>;

struct WriteCommand {
    payload: Vec<u8>,
    result: SyncSender<WriteResult>,
}

/// A shared-memory page-buffer region opened by the host (ADR-009).
struct ShmRegion {
    file: std::fs::File,
    capacity: u64,
    /// Absolute path of the region file, so the host can remove it even when
    /// the worker was killed before its `Drop` could (SIGKILL leaves the file).
    path: PathBuf,
}

/// Positional read at `offset`, filling `buffer` exactly (host side).
#[cfg(unix)]
fn read_exact_at(file: &std::fs::File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.read_at(&mut buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// Positional read at `offset`, filling `buffer` exactly (host side).
#[cfg(windows)]
fn read_exact_at(file: &std::fs::File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.seek_read(&mut buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// Transport over a child process's stdin/stdout pipes.
pub struct ProcessTransport {
    child: Child,
    writer: Option<SyncSender<WriteCommand>>,
    rx: std::sync::mpsc::Receiver<FrameResult>,
    writer_thread: Option<JoinHandle<()>>,
    reader_thread: Option<JoinHandle<()>>,
    response_overflow: Arc<AtomicBool>,
    limits: NegotiatedLimits,
    timeout: Duration,
    /// Shared-memory page-buffer region, when negotiated (ADR-009).
    shm: Option<ShmRegion>,
}

impl ProcessTransport {
    /// Spawn `worker_bin` and perform the `Hello` handshake. Shared-memory
    /// page buffers are enabled (dev `SubprocessBackend` only).
    pub fn spawn(worker_bin: &Path, timeout: Duration) -> Result<Self, SandboxError> {
        let mut command = Command::new(worker_bin);
        Self::spawn_command_inner(&mut command, timeout, true)
    }

    /// Spawn a ready-made command (e.g. `runsc run <id>`) and handshake.
    ///
    /// The command's stdio is piped; its environment is cleared. Shared-memory
    /// page buffers are disabled (the sandbox filesystem is not the host's).
    pub fn spawn_command(command: &mut Command, timeout: Duration) -> Result<Self, SandboxError> {
        Self::spawn_command_inner(command, timeout, false)
    }

    fn spawn_command_inner(
        command: &mut Command,
        timeout: Duration,
        shm_capable: bool,
    ) -> Result<Self, SandboxError> {
        // Harden the child: empty environment (no secrets, no locale tricks).
        // stderr is discarded, never inherited: the untrusted worker (and
        // runsc) could otherwise inject terminal escape sequences into the
        // host console. Errors surface through the protocol instead.
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear()
            .spawn()
            .map_err(|e| {
                tracing::error!("cannot spawn worker command: {e}");
                SandboxError::BackendUnsupported("renderer worker binary unavailable")
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SandboxError::Internal("worker stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SandboxError::Internal("worker stdout unavailable".into()))?;

        Self::from_child_io(
            child,
            Box::new(stdin),
            Box::new(stdout),
            timeout,
            shm_capable,
        )
    }

    /// Spawn QEMU and connect its virtio chardev over a loopback TCP socket.
    ///
    /// Shared-memory page buffers are disabled (the guest is a separate
    /// machine; page bytes travel over the transport).
    pub fn spawn_command_over_tcp(
        command: &mut Command,
        listener: TcpListener,
        timeout: Duration,
    ) -> Result<Self, SandboxError> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .env_clear();
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
            for name in ["SystemRoot", "WINDIR"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
        let mut child = command.spawn().map_err(|e| {
            tracing::error!("cannot spawn QEMU: {e}");
            SandboxError::BackendUnsupported("QEMU binary unavailable")
        })?;

        listener.set_nonblocking(true)?;
        let deadline = std::time::Instant::now() + timeout;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if child.try_wait()?.is_some() {
                        return Err(SandboxError::WorkerCrashed);
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(SandboxError::Timeout);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.into());
                }
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        let reader = stream.try_clone()?;
        Self::from_child_io(child, Box::new(stream), Box::new(reader), timeout, false)
    }

    fn from_child_io(
        child: Child,
        writer: BoxedWriter,
        reader: BoxedReader,
        timeout: Duration,
        shm_capable: bool,
    ) -> Result<Self, SandboxError> {
        let (write_tx, write_rx) = std::sync::mpsc::sync_channel(1);
        let writer_thread = std::thread::spawn(move || write_loop(writer, write_rx));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let response_overflow = Arc::new(AtomicBool::new(false));
        let reader_overflow = Arc::clone(&response_overflow);
        let reader_thread = std::thread::spawn(move || read_loop(reader, tx, reader_overflow));

        let mut transport = Self {
            child,
            writer: Some(write_tx),
            rx,
            writer_thread: Some(writer_thread),
            reader_thread: Some(reader_thread),
            response_overflow,
            limits: NegotiatedLimits {
                max_document_size: 0,
                max_ipc_message: 0,
                max_page_width: 0,
                max_page_height: 0,
                max_pixel_buffer: 0,
            },
            timeout,
            shm: None,
        };

        // Handshake.
        let hello = HelloRequest {
            protocol_version: PROTOCOL_VERSION,
            max_document_size: domain_limits::MAX_DOCUMENT_SIZE as u64,
            max_ipc_message: MAX_IPC_MESSAGE as u32,
            shm_capable,
        };
        match transport.send_raw(Request::Hello(hello))? {
            Response::HelloOk(ok) => {
                transport.limits = match NegotiatedLimits::client_negotiate(&ok) {
                    Ok(limits) => limits,
                    Err(error) => {
                        transport.terminate();
                        return Err(error.into());
                    }
                };
                // Open the shared page-buffer region the worker advertised.
                if let (Some(name), Some(capacity)) = (&ok.shm_name, ok.shm_capacity) {
                    let path = std::env::temp_dir().join(name);
                    // The name itself is strictly validated upstream
                    // (`is_valid_shm_name`), so it cannot traverse the
                    // directory; on POSIX we additionally refuse symlinks
                    // (`O_NOFOLLOW`) and non-blocking open so a swapped
                    // FIFO/pipe cannot stall the host thread.
                    #[cfg(unix)]
                    let opened = {
                        use std::os::unix::fs::OpenOptionsExt;
                        let mut options = std::fs::OpenOptions::new();
                        options
                            .read(true)
                            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                        options.open(&path)
                    };
                    #[cfg(not(unix))]
                    let opened = std::fs::OpenOptions::new().read(true).open(&path);
                    // The worker creates the region before advertising it, but
                    // on hosted/AV-monitored machines a brand-new file in the
                    // temp directory can transiently fail to open (sharing
                    // violation while a scanner holds it). Retry briefly.
                    let opened = match opened {
                        Ok(file) => Ok(file),
                        Err(error) => {
                            const ATTEMPTS: usize = 4;
                            const BACKOFF: std::time::Duration =
                                std::time::Duration::from_millis(50);
                            let mut result = Err(error);
                            for _ in 0..ATTEMPTS - 1 {
                                std::thread::sleep(BACKOFF);
                                #[cfg(unix)]
                                let retry = {
                                    use std::os::unix::fs::OpenOptionsExt;
                                    let mut options = std::fs::OpenOptions::new();
                                    options
                                        .read(true)
                                        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                                    options.open(&path)
                                };
                                #[cfg(not(unix))]
                                let retry = std::fs::OpenOptions::new().read(true).open(&path);
                                match retry {
                                    Ok(file) => {
                                        result = Ok(file);
                                        break;
                                    }
                                    Err(retry_error) => result = Err(retry_error),
                                }
                            }
                            result
                        }
                    };
                    match opened
                        .and_then(|file| file.metadata().map(|metadata| (file, metadata.is_file())))
                    {
                        Ok((file, true)) => {
                            let actual = file.metadata().map(|m| m.len()).unwrap_or(0);
                            if actual < capacity {
                                tracing::warn!(%name, "shm region smaller than advertised");
                                transport.terminate();
                                return Err(SandboxError::Internal(
                                    "worker advertised an invalid shm region".into(),
                                ));
                            }
                            transport.shm = Some(ShmRegion {
                                file,
                                capacity,
                                path,
                            });
                        }
                        Ok((_, false)) => {
                            tracing::warn!(%name, "shm region is not a regular file");
                            transport.terminate();
                            return Err(SandboxError::Internal(
                                "worker advertised an invalid shm region".into(),
                            ));
                        }
                        Err(error) => {
                            tracing::warn!(%error, %name, "cannot open worker shm region");
                            transport.terminate();
                            return Err(SandboxError::Internal(format!(
                                "worker advertised an unusable shm region: {error}"
                            )));
                        }
                    }
                }
                Ok(transport)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            other => Err(SandboxError::Internal(format!(
                "unexpected response to hello: {other:?}"
            ))),
        }
    }

    /// Send a request and receive the response, enforcing one end-to-end deadline.
    ///
    /// On timeout or crash the child is terminated (the stream is unusable).
    fn send_raw(&mut self, request: Request) -> Result<Response, SandboxError> {
        if self.response_overflow.load(Ordering::Acquire) {
            self.terminate();
            return Err(SandboxError::Internal(
                "worker sent unsolicited responses".into(),
            ));
        }
        match self.rx.try_recv() {
            Ok(_) => {
                self.terminate();
                return Err(SandboxError::Internal(
                    "worker sent an unsolicited response".into(),
                ));
            }
            Err(TryRecvError::Disconnected) => {
                self.terminate();
                return Err(SandboxError::WorkerCrashed);
            }
            Err(TryRecvError::Empty) => {}
        }

        let deadline = Instant::now()
            .checked_add(self.timeout)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        let payload = encode_request(&request)?;
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
        let mut command = WriteCommand {
            payload,
            result: result_tx,
        };
        let Some(writer) = self.writer.as_ref() else {
            self.terminate();
            return Err(SandboxError::Internal("transport already closed".into()));
        };
        // Enqueue within the same deadline: `send` would block forever if the
        // writer thread is stuck on a full pipe, leaving the deadline unused.
        loop {
            match writer.try_send(command) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        self.terminate();
                        return Err(SandboxError::Timeout);
                    }
                    command = returned;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.terminate();
                    return Err(SandboxError::WorkerCrashed);
                }
            }
        }

        let remaining = self.remaining(deadline)?;
        match result_rx.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.terminate();
                return Err(SandboxError::Internal(format!(
                    "worker write error: {error}"
                )));
            }
            Err(RecvTimeoutError::Timeout) => {
                self.terminate();
                return Err(SandboxError::Timeout);
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.terminate();
                return Err(SandboxError::WorkerCrashed);
            }
        }

        let remaining = self.remaining(deadline)?;
        let frame = match self.rx.recv_timeout(remaining) {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) => {
                self.terminate();
                return Err(SandboxError::WorkerCrashed);
            }
            Ok(Err(e)) => {
                self.terminate();
                return Err(SandboxError::Internal(format!("worker read error: {e}")));
            }
            Err(RecvTimeoutError::Timeout) => {
                self.terminate();
                return Err(SandboxError::Timeout);
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.terminate();
                return Err(SandboxError::WorkerCrashed);
            }
        };

        let response = match decode_response(&frame) {
            Ok(response) => response,
            Err(error) => {
                self.terminate();
                return Err(error.into());
            }
        };
        // Shared-memory page buffers (ADR-009): copy the raster out of the
        // region and restore the legacy in-frame shape **before** validation,
        // so the rest of the trust chain is byte-for-byte unchanged.
        let response = match self.normalize_shm(response) {
            Ok(response) => response,
            Err(error) => {
                self.terminate();
                return Err(error);
            }
        };
        if let Err(error) = self.validate_response(&request, &response) {
            self.terminate();
            return Err(error);
        }

        if self.response_overflow.load(Ordering::Acquire) || self.rx.try_recv().is_ok() {
            self.terminate();
            return Err(SandboxError::Internal(
                "worker sent an unsolicited extra response".into(),
            ));
        }
        Ok(response)
    }

    /// Copy a shared-memory page raster into the response, when present.
    fn normalize_shm(&self, mut response: Response) -> Result<Response, SandboxError> {
        let Response::PageRendered(ref mut page) = response else {
            return Ok(response);
        };
        let Some(shm_len) = page.shm_len.take() else {
            return Ok(response);
        };
        let region = self.shm.as_ref().ok_or_else(|| {
            SandboxError::Internal("worker sent a shm page without a region".into())
        })?;
        let len = usize::try_from(shm_len)
            .map_err(|_| SandboxError::Internal("shm length out of range".into()))?;
        if len > region.capacity as usize || len > self.limits.max_pixel_buffer {
            return Err(SandboxError::Internal(
                "worker sent a shm page larger than negotiated".into(),
            ));
        }
        let mut bytes = vec![0u8; len];
        // Publish fence paired with the worker's release fence (ADR-009).
        std::sync::atomic::fence(Ordering::Acquire);
        if len > 0 {
            read_exact_at(&region.file, &mut bytes, 0).map_err(|error| {
                SandboxError::Internal(format!("cannot read worker shm region: {error}"))
            })?;
        }
        page.bytes = bytes;
        Ok(response)
    }

    fn remaining(&mut self, deadline: Instant) -> Result<Duration, SandboxError> {
        match deadline.checked_duration_since(Instant::now()) {
            Some(remaining) if !remaining.is_zero() => Ok(remaining),
            _ => {
                self.terminate();
                Err(SandboxError::Timeout)
            }
        }
    }

    fn validate_response(
        &self,
        request: &Request,
        response: &Response,
    ) -> Result<(), SandboxError> {
        if let Response::Error(error) = response {
            validate_error_response(error)?;
            return Ok(());
        }

        let matches = match (request, response) {
            (Request::Hello(_), Response::HelloOk(_)) => true,
            (Request::OpenDocument(request), Response::DocumentOpened(response)) => {
                self.limits.validate_document_info(&response.info)?;
                request.document_id == response.document_id
            }
            (Request::GetDocumentInfo(request), Response::DocumentInfo(response)) => {
                self.limits.validate_document_info(&response.info)?;
                request.document_id == response.document_id
            }
            (Request::RenderPage(request), Response::PageRendered(response)) => {
                self.limits.validate_rendered_page(response)?;
                request.document_id == response.document_id && request.page == response.page
            }
            (Request::CloseDocument(request), Response::Closed(response)) => {
                request.document_id == response.document_id
            }
            (Request::Ping(request), Response::Pong(response)) => request.nonce == response.nonce,
            _ => false,
        };
        if matches {
            Ok(())
        } else {
            Err(SandboxError::Internal(format!(
                "worker response does not match request: {response:?}"
            )))
        }
    }

    /// Close stdin (graceful EOF path), wait briefly, then kill if needed.
    pub fn shutdown(&mut self) {
        self.writer.take();
        self.reap(true);
    }

    /// Kill and reap the child; join the reader thread.
    pub fn terminate(&mut self) {
        self.writer.take();
        let _ = self.child.kill();
        self.reap(false);
    }

    fn reap(&mut self, allow_grace: bool) {
        if allow_grace {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    _ => {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        break;
                    }
                }
            }
        } else {
            let _ = self.child.wait();
        }
        // Join the pipe threads, but never block forever: a descendant that
        // inherited the stdout pipe keeps the reader thread alive even after
        // the direct child is reaped. Wait briefly, then detach (logged).
        let mut threads = [self.reader_thread.take(), self.writer_thread.take()];
        let join_deadline = std::time::Instant::now() + Duration::from_secs(1);
        for handle in threads.iter_mut().flatten() {
            while !handle.is_finished() && std::time::Instant::now() < join_deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        for handle in threads.into_iter().flatten() {
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                tracing::warn!("transport pipe thread did not finish; detaching");
            }
        }
        // Remove the shared page-buffer file the worker may have left behind:
        // worker-side removal only runs on graceful `Drop`, so a worker killed
        // with SIGKILL would otherwise leak the file (and its content).
        if let Some(shm) = self.shm.take() {
            let _ = std::fs::remove_file(&shm.path);
        }
    }
}

fn write_loop(writer: BoxedWriter, rx: std::sync::mpsc::Receiver<WriteCommand>) {
    let mut writer = FrameWriter::new(BufWriter::new(writer), MAX_IPC_MESSAGE);
    while let Ok(command) = rx.recv() {
        let result = writer
            .write_frame(&command.payload)
            .and_then(|()| writer.flush())
            .map_err(|error| error.to_string());
        let failed = result.is_err();
        let _ = command.result.send(result);
        if failed {
            return;
        }
    }
}

fn read_loop(stdout: BoxedReader, tx: SyncSender<FrameResult>, overflow: Arc<AtomicBool>) {
    let mut reader = FrameReader::new(BufReader::new(stdout), MAX_IPC_MESSAGE);
    loop {
        match reader.next_frame() {
            Ok(Some(frame)) => match tx.try_send(Ok(Some(frame))) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    overflow.store(true, Ordering::Release);
                    return;
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return,
            },
            Ok(None) => {
                if let Err(std::sync::mpsc::TrySendError::Full(_)) = tx.try_send(Ok(None)) {
                    overflow.store(true, Ordering::Release);
                }
                return;
            }
            Err(e) => {
                if let Err(std::sync::mpsc::TrySendError::Full(_)) = tx.try_send(Err(e.to_string()))
                {
                    overflow.store(true, Ordering::Release);
                }
                return;
            }
        }
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        self.writer.take();
        let _ = self.child.kill();
        self.reap(false);
    }
}

impl SandboxTransport for ProcessTransport {
    fn send(&mut self, request: Request) -> Result<Response, SandboxError> {
        // Host-side pre-validation before the message reaches the worker.
        match &request {
            Request::OpenDocument(open) => self.limits.validate_open_document(open)?,
            Request::RenderPage(render) => self.limits.validate_render_request(render)?,
            _ => {}
        }
        self.send_raw(request)
    }
}

/// Document-level session over a [`ProcessTransport`].
///
/// Shared by the subprocess backend (dev) and the `runsc` backend (Phase 4):
/// the wire plumbing is identical, only the transport differs.
pub struct WorkerSession {
    transport: ProcessTransport,
    docs: HashMap<u64, DocumentInfo>,
}

impl WorkerSession {
    pub fn new(transport: ProcessTransport) -> Self {
        Self {
            transport,
            docs: HashMap::new(),
        }
    }

    pub fn send_document(&mut self, data: Vec<u8>, id: u64) -> Result<DocumentInfo, SandboxError> {
        let response = self
            .transport
            .send(Request::OpenDocument(OpenDocumentRequest {
                document_id: id,
                data,
            }))?;
        match response {
            Response::DocumentOpened(opened) => {
                let info = document_info_from_wire(&opened.info)?;
                self.docs.insert(id, info.clone());
                Ok(info)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to open document".into(),
            )),
        }
    }

    pub fn get_document_info(&mut self, id: u64) -> Result<DocumentInfo, SandboxError> {
        if !self.docs.contains_key(&id) {
            return Err(SandboxError::UnknownDocument);
        }
        let response = self
            .transport
            .send(Request::GetDocumentInfo(GetDocumentInfoRequest {
                document_id: id,
            }))?;
        match response {
            Response::DocumentInfo(info) => {
                let validated = document_info_from_wire(&info.info)?;
                self.docs.insert(id, validated.clone());
                Ok(validated)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to get document info".into(),
            )),
        }
    }

    pub fn render_page(
        &mut self,
        id: u64,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, SandboxError> {
        let response = self.transport.send(Request::RenderPage(RenderPageRequest {
            document_id: id,
            page,
            target_width: options.target_width,
            target_height: options.target_height,
        }))?;
        match response {
            Response::PageRendered(rendered) => {
                let page = rendered_page_from_wire(&rendered)?;
                page.validate()?;
                Ok(page)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to render page".into(),
            )),
        }
    }

    /// Tell the worker to close every open document (best-effort).
    pub fn close_all_documents(&mut self) {
        let ids: Vec<u64> = self.docs.keys().copied().collect();
        for id in ids {
            let response = self
                .transport
                .send(Request::CloseDocument(CloseDocumentRequest {
                    document_id: id,
                }));
            match response {
                Ok(Response::Closed(_)) => {}
                Ok(Response::Error(e)) => {
                    tracing::warn!(code = e.code, "worker refused to close document {id}");
                }
                Ok(other) => tracing::warn!("unexpected close response: {other:?}"),
                Err(e) => {
                    tracing::warn!(%e, "close document {id} failed");
                    return;
                }
            }
        }
        self.docs.clear();
    }

    pub fn shutdown(&mut self) {
        self.transport.shutdown();
    }

    pub fn terminate(&mut self) {
        self.transport.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbunker_protocol::message::{OpenDocumentRequest, PingRequest};

    fn worker_bin() -> std::path::PathBuf {
        let mut path = std::env::current_exe().unwrap();
        path.pop(); // deps/
        path.pop(); // debug/
        path.push(if cfg!(windows) {
            "renderer-worker.exe"
        } else {
            "renderer-worker"
        });
        path
    }

    #[test]
    fn spawn_fails_for_missing_binary() {
        let result =
            ProcessTransport::spawn(Path::new("definitely-not-a-binary"), Duration::from_secs(2));
        assert!(matches!(result, Err(SandboxError::BackendUnsupported(_))));
    }

    #[test]
    fn handshake_and_ping() {
        let mut t = ProcessTransport::spawn(&worker_bin(), Duration::from_secs(5)).unwrap();
        match t.send(Request::Ping(PingRequest { nonce: 3 })).unwrap() {
            Response::Pong(p) => assert_eq!(p.nonce, 3),
            other => panic!("expected Pong, got {other:?}"),
        }
        t.terminate();
    }

    #[test]
    fn oversized_open_rejected_before_worker() {
        let mut t = ProcessTransport::spawn(&worker_bin(), Duration::from_secs(5)).unwrap();
        let result = t.send(Request::OpenDocument(OpenDocumentRequest {
            document_id: 1,
            data: vec![0; domain_limits::MAX_DOCUMENT_SIZE + 1],
        }));
        assert!(matches!(
            result,
            Err(SandboxError::Protocol(
                docbunker_protocol::ProtocolError::LimitViolation(_)
            ))
        ));
        t.terminate();
    }

    #[test]
    fn graceful_shutdown_closes_child() {
        let mut t = ProcessTransport::spawn(&worker_bin(), Duration::from_secs(5)).unwrap();
        t.shutdown();
    }
}
