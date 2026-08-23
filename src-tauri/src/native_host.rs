//! Chrome native-messaging host **registration**.
//!
//! The inbound native-message loop lives in the dedicated `docbunker-native-broker`
//! binary (`crates/native-broker`), which is the only component the Chrome
//! extension can reach. The app only writes the host manifest (pointing at the
//! broker) and the registry keys — and verifies the broker is actually
//! deployed next to it so we never register a dangling host.

#[cfg(any(target_os = "windows", test))]
use std::path::PathBuf;

use docbunker_native_broker::{EXTENSION_ORIGIN, HOST_NAME};

/// Locate the broker binary (a sibling of the app executable).
///
/// Release packaging must ship `docbunker-native-broker` next to the app;
/// `DOCBUNKER_NATIVE_BROKER_BIN` overrides it for developers.
#[cfg(any(target_os = "windows", test))]
pub fn broker_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "docbunker-native-broker.exe"
    } else {
        "docbunker-native-broker"
    };
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|dir| dir.join(name)))
}

/// The broker must exist before we point the browser at it; otherwise Chrome
/// would either fail loudly (misconfiguration) or, worse, invoke nothing.
#[cfg(any(target_os = "windows", test))]
fn broker_binary_or_error() -> Result<PathBuf, String> {
    broker_binary()
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            "DocBunker native-messaging broker is missing (ship docbunker-native-broker next to the app)"
                .to_string()
        })
}

#[cfg(target_os = "windows")]
pub fn register() -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let broker = broker_binary_or_error()?;
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable".to_string())?
        .join("DocBunker")
        .join("Chrome");
    std::fs::create_dir_all(&base)
        .map_err(|error| format!("cannot create Chrome integration directory: {error}"))?;
    let manifest_path = base.join(format!("{HOST_NAME}.json"));
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "Open downloaded Gmail attachments in DocBunker",
        "path": broker,
        "type": "stdio",
        "allowed_origins": [EXTENSION_ORIGIN],
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write native host manifest: {error}"))?;

    let chrome = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = chrome
        .create_subkey(format!(
            "Software\\Google\\Chrome\\NativeMessagingHosts\\{HOST_NAME}"
        ))
        .map_err(|error| format!("cannot register Chrome native host: {error}"))?;
    key.set_value("", &manifest_path.to_string_lossy().as_ref())
        .map_err(|error| format!("cannot register native host manifest: {error}"))
}

#[cfg(not(target_os = "windows"))]
pub fn register() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_name_is_platform_consistent() {
        let name = if cfg!(windows) {
            "docbunker-native-broker.exe"
        } else {
            "docbunker-native-broker"
        };
        let executable = std::env::current_exe().unwrap();
        let expected = executable.parent().unwrap().join(name);
        let previous = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN");
        std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN");
        assert_eq!(broker_binary().as_ref(), Some(&expected));
        match previous {
            Some(value) => std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", value),
            None => std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN"),
        }
    }

    #[test]
    fn broker_override_is_honored() {
        let path = PathBuf::from("C:\\DocBunker\\broker.exe");
        let previous = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN");
        std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", &path);
        assert_eq!(broker_binary().as_deref(), Some(path.as_path()));
        match previous {
            Some(value) => std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", value),
            None => std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN"),
        }
    }
}
