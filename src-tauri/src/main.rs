//! DocBunker Tauri shell entry point.
//!
//! Trusted host code. It wires the Rust core to the UI through explicit
//! commands and runs all backend work on blocking threads so the UI never
//! blocks.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_config;
mod commands;
mod native_host;

use std::sync::Arc;

use app_config::{AppConfig, BackendConfig};
use commands::{AppState, StartupFiles};
use docbunker_sandbox::{MockBackend, SandboxBackend, SubprocessBackend};
use tauri::{path::BaseDirectory, Emitter, Manager};

/// Select the sandbox backend.
fn select_backend(config: AppConfig) -> Result<Box<dyn SandboxBackend>, String> {
    match config.backend {
        BackendConfig::Mock => {
            tracing::warn!("backend: mock — mock pages, no parsing, no isolation");
            Ok(Box::new(MockBackend::new()))
        }
        BackendConfig::Subprocess { worker_bin } => {
            tracing::warn!("backend: subprocess — NO isolation, development only");
            Ok(Box::new(SubprocessBackend::new(worker_bin)))
        }
        BackendConfig::Runsc(config) => {
            #[cfg(target_os = "linux")]
            {
                use docbunker_sandbox::platforms::RunscBackend;
                tracing::info!("backend: runsc (gVisor)");
                Ok(Box::new(RunscBackend::new(config)))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = config;
                Err("runsc backend requires Linux".into())
            }
        }
        BackendConfig::Vm(config) => {
            use docbunker_sandbox::platforms::QemuVmBackend;
            tracing::info!(accelerator = %config.accel, "backend: vm (qemu)");
            Ok(Box::new(QemuVmBackend::new(config)))
        }
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let startup_file = app_config::pending_document_from_args(arguments);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .manage(StartupFiles(std::sync::Mutex::new(
            startup_file.into_iter().collect(),
        )))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _| {
            if let Some(pending) = app_config::pending_document_from_args(
                arguments.into_iter().map(std::ffi::OsString::from),
            ) {
                let state = app.state::<StartupFiles>();
                let queued = state
                    .0
                    .lock()
                    .map(|mut queue| {
                        if queue.len() >= 4 {
                            if let Some(ack_path) = &pending.ack_path {
                                let _ = std::fs::write(ack_path, "error");
                            }
                            return false;
                        }
                        queue.push_back(pending);
                        true
                    })
                    .unwrap_or(false);
                if queued {
                    let _ = app.emit("associated-file-ready", ());
                }
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("navigation-guard")
                .on_navigation(|_, url| {
                    url.scheme() == "tauri"
                        || url.host_str() == Some("tauri.localhost")
                        || (cfg!(debug_assertions)
                            && matches!(url.host_str(), Some("localhost" | "127.0.0.1")))
                })
                .build(),
        )
        .setup(move |app| {
            if let Err(error) = native_host::register() {
                tracing::warn!(%error, "failed to register native messaging host");
            }

            // Register docbunker:// custom protocol
            if let Err(error) = app.deep_link().register("docbunker") {
                tracing::warn!(%error, "failed to register docbunker:// protocol");
            }

            // Handle docbunker:// URLs (e.g. docbunker://open?path=/path/to/file.pdf)
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if url.scheme() == "docbunker" && url.host_str() == Some("open") {
                        if let Some(path) = url.query_pairs().find_map(|(k, v)| {
                            if k == "path" {
                                Some(v.into_owned())
                            } else {
                                None
                            }
                        }) {
                            let path = std::path::PathBuf::from(&path);
                            if path.is_file() {
                                let state = app_handle.state::<StartupFiles>();
                                if let Ok(mut queue) = state.0.lock() {
                                    let pending = app_config::PendingDocument {
                                        path,
                                        ack_path: None,
                                    };
                                    if queue.len() < 4 {
                                        queue.push_back(pending);
                                        let _ = app_handle.emit("associated-file-ready", ());
                                    }
                                }
                            }
                        }
                    }
                }
            });
            let vm_dir = app.path().resolve("vm", BaseDirectory::Resource).ok();
            let config = AppConfig::from_env(vm_dir.as_deref());
            let configured_isolation = config
                .as_ref()
                .map(|config| config.backend.is_isolated())
                .unwrap_or(false);
            let backend = config.and_then(select_backend);
            let (manager, backend_isolated) = match backend {
                Ok(backend) => match docbunker_core::DocumentManager::new(backend) {
                    Ok(manager) => (Some(Arc::new(manager)), configured_isolation),
                    Err(e) => {
                        tracing::error!(%e, "failed to initialize the document manager");
                        (None, false)
                    }
                },
                Err(e) => {
                    tracing::error!(%e, "failed to configure the sandbox backend");
                    (None, false)
                }
            };
            app.manage(AppState {
                manager,
                backend_isolated,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_backend_status,
            commands::open_startup_document,
            commands::open_document,
            commands::open_document_by_path,
            commands::get_document_info,
            commands::render_page,
            commands::close_document,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DocBunker");
}
