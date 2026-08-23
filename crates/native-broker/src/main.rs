//! DocBunker native-messaging broker.
//!
//! A deliberately minimal binary that is the *only* component the Chrome
//! extension can reach. It must never grow a UI, a WebView, or a document
//! parser: the whole trusted surface it exposes is "open this one file that I
//! already proved is a supported, size-bounded document inside the user's
//! Downloads folder".
//!
//! The broker:
//!
//! 1. reads one Chrome native message over anonymous pipes (length-prefixed);
//! 2. validates the request (action, signature, size, path containment);
//! 3. launches the DocBunker app with the path plus an ack file and waits for
//!    the app to acknowledge (or refuse) the hand-off;
//! 4. answers the browser with `{ok, message}`.
//!
//! The app is located as a sibling binary (`docbunker-app`), overridable with
//! `DOCBUNKER_APP_BIN` for development.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use docbunker_native_broker::{validate_document_path, MAX_NATIVE_MESSAGE_SIZE};

fn app_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DOCBUNKER_APP_BIN") {
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let name = if cfg!(windows) {
        "docbunker-app.exe"
    } else {
        "docbunker-app"
    };
    let sibling = executable
        .parent()
        .map(|dir| dir.join(name))
        .ok_or_else(|| "cannot locate DocBunker executable".to_string())?;
    Ok(sibling)
}

fn run() -> Result<(), String> {
    // Chrome launches native hosts over anonymous pipes. A terminal-driven
    // invocation is never the browser; whatever local process is driving it,
    // the negotiated file-open primitive should not be exposed interactively.
    if std::io::stdin().is_terminal() {
        return Err("native host must be launched by the browser over pipes".to_string());
    }
    let mut length = [0; 4];
    std::io::stdin()
        .read_exact(&mut length)
        .map_err(|error| format!("cannot read native message length: {error}"))?;
    let length = u32::from_ne_bytes(length) as usize;
    if length == 0 || length > MAX_NATIVE_MESSAGE_SIZE {
        return write_response(false, "invalid message size");
    }

    let mut payload = vec![0; length];
    std::io::stdin()
        .read_exact(&mut payload)
        .map_err(|error| format!("cannot read native message: {error}"))?;
    let request: Request = serde_json::from_slice(&payload)
        .map_err(|error| format!("invalid native message: {error}"))?;

    if request.action != "open_file" {
        return write_response(false, "unsupported request");
    }
    let path = Path::new(&request.path);
    if let Err(message) = validate_document_path(path) {
        return write_response(false, message);
    }

    let app = app_binary()?;
    let ack_dir = tempfile::Builder::new()
        .prefix("docbunker-native-")
        .tempdir()
        .map_err(|error| format!("cannot create handoff state: {error}"))?;
    let ack_path = ack_dir.path().join("result");
    std::process::Command::new(&app)
        .arg(path)
        .arg(format!("--docbunker-ack={}", ack_path.display()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot launch DocBunker: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        match std::fs::read_to_string(&ack_path) {
            Ok(status) if status == "ok" => return write_response(true, "opened"),
            Ok(_) => return write_response(false, "DocBunker rejected the attachment"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(format!("cannot read DocBunker acknowledgment: {error}")),
        }
    }
    write_response(false, "DocBunker did not acknowledge the attachment")
}

fn write_response(ok: bool, message: &str) -> Result<(), String> {
    let payload = serde_json::to_vec(&Response { ok, message })
        .map_err(|error| format!("cannot encode native response: {error}"))?;
    let length = u32::try_from(payload.len()).map_err(|_| "native response too large")?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&length.to_ne_bytes())
        .and_then(|_| stdout.write_all(&payload))
        .and_then(|_| stdout.flush())
        .map_err(|error| format!("cannot write native response: {error}"))
}

#[derive(serde::Deserialize)]
struct Request {
    action: String,
    path: String,
}

#[derive(serde::Serialize)]
struct Response<'a> {
    ok: bool,
    message: &'a str,
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(%error, "native messaging error");
            let _ = write_response(false, &error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locates_app_next_to_broker() {
        let previous_bin = std::env::var_os("DOCBUNKER_APP_BIN");
        std::env::remove_var("DOCBUNKER_APP_BIN");
        let result = app_binary();
        assert!(
            result.is_ok(),
            "broker must be built next to the app or DOCBUNKER_APP_BIN set"
        );
        let binary = result.unwrap();
        assert!(binary.is_absolute());
        assert!(binary.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name == "docbunker-app" || name == "docbunker-app.exe"
        }));
        match previous_bin {
            Some(value) => std::env::set_var("DOCBUNKER_APP_BIN", value),
            None => std::env::remove_var("DOCBUNKER_APP_BIN"),
        }
    }

    #[test]
    fn env_override_for_app_binary() {
        let path = std::path::PathBuf::from("C:\\DocBunker\\app.exe");
        std::env::set_var("DOCBUNKER_APP_BIN", &path);
        assert_eq!(app_binary().unwrap(), path);
        std::env::remove_var("DOCBUNKER_APP_BIN");
    }
}
