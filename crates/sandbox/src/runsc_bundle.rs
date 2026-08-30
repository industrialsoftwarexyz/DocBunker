//! OCI bundle generation for the `runsc` backend (Phase 4).
//!
//! Platform-independent: building and inspecting the OCI config is unit
//! testable on every host; only actually spawning `runsc` is Linux-only
//! (`crates/sandbox/src/platforms/linux.rs`).
//!
//! Hardening properties encoded here (threat model A10–A14, A19, A21):
//!
//! - read-only rootfs, no host mounts
//! - unprivileged user (65534), `no_new_privileges`, no capabilities
//! - empty network namespace (no connectivity)
//! - private size-capped tmpfs `/tmp`
//! - cgroup limits: memory, CPU, pids
//! - minimal rlimits

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::SandboxConfig;
use crate::error::SandboxError;
use serde_json::{json, Value};

/// The namespace types the sandbox must create.
const NAMESPACES: [&str; 5] = ["pid", "network", "ipc", "uts", "mount"];

/// The default (production) container init: the renderer worker.
const DEFAULT_ARGS: [&str; 1] = ["/bin/renderer-worker"];

/// The runtime user (nobody).
const SANDBOX_UID: u32 = 65534;
const SANDBOX_GID: u32 = 65534;

/// Size cap for the sandbox `/tmp` tmpfs.
const TMPFS_TMP_SIZE: u64 = 256 * 1024 * 1024;

/// A prepared OCI bundle: `config.json` on disk + the rootfs it points at.
#[derive(Debug)]
pub struct OciBundle {
    pub bundle_dir: PathBuf,
    /// Absolute path of the read-only rootfs shared by sessions.
    pub rootfs_dir: PathBuf,
}

impl OciBundle {
    /// Write `config.json` into a fresh bundle directory.
    ///
    /// `rootfs_dir` is used as-is (read-only, shared across sessions); the
    /// bundle directory itself only contains the config.
    pub fn write(
        bundle_dir: &Path,
        rootfs_dir: &Path,
        config: &SandboxConfig,
    ) -> Result<(), SandboxError> {
        Self::write_with_args(
            bundle_dir,
            rootfs_dir,
            config,
            &DEFAULT_ARGS.map(String::from),
        )
    }

    /// Write `config.json` with a custom process argv.
    ///
    /// The hardening profile (read-only rootfs, no capabilities, unprivileged
    /// user, no network, cgroup limits) is identical to [`OciBundle::write`];
    /// only the init binary and its arguments change. This exists so sandbox
    /// **escape tests** can run a deliberately malicious init against the
    /// exact same OCI configuration that production uses.
    pub fn write_with_args(
        bundle_dir: &Path,
        rootfs_dir: &Path,
        config: &SandboxConfig,
        args: &[String],
    ) -> Result<(), SandboxError> {
        validate_bundle_dir(bundle_dir)?;
        let rootfs = rootfs_dir.canonicalize().map_err(|e| {
            tracing::error!(path = %rootfs_dir.display(), "rootfs not usable: {e}");
            SandboxError::BackendUnsupported("sandbox rootfs unavailable")
        })?;

        let value = build_config(&rootfs, config, args);
        let rendered = serde_json::to_string_pretty(&value)
            .map_err(|e| SandboxError::Internal(format!("cannot render oci config: {e}")))?;
        write_config_exclusive(bundle_dir, rendered.as_bytes())?;
        Ok(())
    }

    /// Write `config.json` for a bundle that lives **inside a VM guest**.
    ///
    /// Identical hardening to [`OciBundle::write`], but `rootfs_in_guest` is
    /// used verbatim (no host-side canonicalization): the bundle is generated
    /// at image build time on the CI host, while the path only exists inside
    /// the guest's initramfs (e.g. `/bundle/rootfs`).
    pub fn write_with_guest_rootfs(
        bundle_dir: &Path,
        rootfs_in_guest: &Path,
        config: &SandboxConfig,
    ) -> Result<(), SandboxError> {
        validate_bundle_dir(bundle_dir)?;
        let value = build_config(rootfs_in_guest, config, &DEFAULT_ARGS.map(String::from));
        let rendered = serde_json::to_string_pretty(&value)
            .map_err(|e| SandboxError::Internal(format!("cannot render oci config: {e}")))?;
        write_config_exclusive(bundle_dir, rendered.as_bytes())?;
        Ok(())
    }
}

fn validate_bundle_dir(bundle_dir: &Path) -> Result<(), SandboxError> {
    let metadata = match std::fs::symlink_metadata(bundle_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = std::fs::DirBuilder::new();
                builder.mode(0o700);
                builder.create(bundle_dir)?;
            }
            #[cfg(not(unix))]
            std::fs::DirBuilder::new().create(bundle_dir)?;
            std::fs::symlink_metadata(bundle_dir)?
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SandboxError::Internal(
            "OCI bundle path must be a real directory".into(),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o700 {
            return Err(SandboxError::Internal(
                "OCI bundle directory must have mode 0700".into(),
            ));
        }
    }
    Ok(())
}

fn write_config_exclusive(bundle_dir: &Path, rendered: &[u8]) -> Result<(), SandboxError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(bundle_dir.join("config.json"))?;
    file.write_all(rendered)?;
    file.sync_all()?;
    Ok(())
}

/// Build the OCI runtime-spec config (version 1.0.2).
fn build_config(rootfs: &Path, config: &SandboxConfig, args: &[String]) -> Value {
    let resources = build_resources(config);
    let mounts = build_mounts(config);
    let env = ["PATH=/bin", "HOME=/"];
    let args: Vec<Value> = args.iter().map(|a| json!(a)).collect();

    json!({
        "ociVersion": "1.0.2",
        "process": {
            "terminal": false,
            "user": { "uid": SANDBOX_UID, "gid": SANDBOX_GID },
            "args": args,
            "env": env,
            "cwd": "/tmp",
            "capabilities": {
                "bounding": [],
                "effective": [],
                "permitted": [],
                "inheritable": [],
                "ambient": []
            },
            "rlimits": [
                { "type": "RLIMIT_NOFILE", "hard": 64, "soft": 64 },
                { "type": "RLIMIT_CORE", "hard": 0, "soft": 0 }
            ],
            "noNewPrivileges": true
        },
        "root": {
            "path": rootfs.to_string_lossy(),
            "readonly": true
        },
        "hostname": "docbunker",
        "mounts": mounts,
        "linux": {
            "resources": resources,
            "namespaces": NAMESPACES
                .iter()
                .map(|ns| json!({ "type": ns }))
                .collect::<Vec<_>>(),
            // Explicit empty device list: the runtime's default device set is
            // never relied upon, matching the hardening table (docs/sandbox.md).
            "devices": [],
            "maskedPaths": [
                "/proc/kcore",
                "/proc/latency_stats",
                "/proc/timer_list",
                "/proc/timer_stats",
                "/proc/sched_debug",
                "/proc/scsi",
                "/sys/firmware"
            ],
            "readonlyPaths": [
                "/proc/asound",
                "/proc/bus",
                "/proc/fs",
                "/proc/irq",
                "/proc/sys",
                "/proc/sysrq-trigger"
            ]
        }
    })
}

fn build_mounts(config: &SandboxConfig) -> Vec<Value> {
    let tmp_size = config
        .memory_limit_bytes
        .map(|mem| (mem / 2).min(TMPFS_TMP_SIZE))
        .unwrap_or(TMPFS_TMP_SIZE);
    vec![
        json!({ "destination": "/proc", "type": "proc", "source": "proc", "options": [] }),
        json!({ "destination": "/dev", "type": "tmpfs", "source": "tmpfs",
                "options": ["nosuid", "strictatime", "mode=755", "size=65536k"] }),
        json!({ "destination": "/dev/pts", "type": "devpts", "source": "devpts",
                "options": ["nosuid", "noexec", "newinstance", "ptmxmode=0666", "mode=0620"] }),
        json!({ "destination": "/dev/shm", "type": "tmpfs", "source": "shm",
                "options": ["nosuid", "noexec", "nodev", "mode=1777", "size=65536k"] }),
        json!({ "destination": "/tmp", "type": "tmpfs", "source": "tmpfs",
                "options": ["nosuid", "noexec", "nodev", "size=262144k", "mode=0700"] }),
    ]
    .into_iter()
    .map(|mut m: Value| {
        // Override the /tmp size cap from the session config.
        if m["destination"] == "/tmp" {
            if let Some(options) = m["options"].as_array_mut() {
                if let Some(size) = options
                    .iter_mut()
                    .find(|o| o.as_str().is_some_and(|s| s.starts_with("size=")))
                {
                    *size = json!(format!("size={}k", tmp_size / 1024));
                }
            }
        }
        m
    })
    .collect()
}

fn build_resources(config: &SandboxConfig) -> Value {
    let mut resources = BTreeMap::new();

    if let Some(memory) = config.memory_limit_bytes {
        resources.insert("memory", json!({ "limit": memory, "swap": 0 }));
    }
    if let Some(millicpus) = config.cpu_limit_millicpus {
        // cfs quota/period: quota = millicpus * period / 1000
        let period: u64 = 100_000;
        let quota = (u128::from(millicpus) * u128::from(period) / 1000) as u64;
        resources.insert("cpu", json!({ "quota": quota, "period": period }));
    }
    if let Some(pids) = config.max_processes {
        resources.insert("pids", json!({ "limit": pids }));
    }

    json!(resources)
}

/// Parse `runsc --version` output: `runsc version <release> <commit>`.
pub fn parse_runsc_version(output: &str) -> Option<String> {
    let rest = output.trim().strip_prefix("runsc")?.trim();
    let rest = rest.strip_prefix("version")?.trim();
    let version = rest.split_whitespace().next()?.to_string();
    Some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle_dir(prefix: &str) -> tempfile::TempDir {
        let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        // OciBundle::write refuses directories looser than 0700; do not depend
        // on tempfile's platform-specific default permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
                .expect("set bundle dir mode");
        }
        dir
    }

    fn sample_config() -> SandboxConfig {
        SandboxConfig {
            memory_limit_bytes: Some(536_870_912),
            cpu_limit_millicpus: Some(1000),
            max_processes: Some(64),
            ..SandboxConfig::default()
        }
    }

    #[test]
    fn renders_config_with_hardening() {
        let dir = bundle_dir("docbunker-bundle-test-");
        let rootfs = std::env::temp_dir();
        OciBundle::write(dir.path(), &rootfs, &sample_config()).unwrap();

        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();

        // Read-only rootfs pointing at the absolute rootfs path.
        assert_eq!(value["root"]["readonly"], true);
        let canonical_rootfs = rootfs.canonicalize().unwrap();
        assert_eq!(
            value["root"]["path"],
            canonical_rootfs.to_string_lossy().as_ref()
        );

        // Unprivileged user, no capabilities, no_new_privileges.
        assert_eq!(value["process"]["user"]["uid"], 65534);
        assert_eq!(value["process"]["user"]["gid"], 65534);
        assert!(value["process"]["noNewPrivileges"] == json!(true));
        assert_eq!(value["process"]["capabilities"]["bounding"], json!([]));
        assert_eq!(value["process"]["capabilities"]["effective"], json!([]));
        assert_eq!(value["process"]["capabilities"]["permitted"], json!([]));
        assert_eq!(value["process"]["capabilities"]["ambient"], json!([]));

        // Network namespace present (no connectivity) + others.
        let namespaces: Vec<&str> = value["linux"]["namespaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["type"].as_str())
            .collect();
        assert!(namespaces.contains(&"network"));
        assert!(namespaces.contains(&"pid"));
        assert!(namespaces.contains(&"mount"));

        // Resources from the session config.
        assert_eq!(value["linux"]["resources"]["memory"]["limit"], 536_870_912);
        assert_eq!(value["linux"]["resources"]["cpu"]["quota"], 100_000);
        assert_eq!(value["linux"]["resources"]["pids"]["limit"], 64);

        // Worker as init, minimal env.
        assert_eq!(value["process"]["args"], json!(["/bin/renderer-worker"]));
        assert_eq!(value["process"]["env"], json!(["PATH=/bin", "HOME=/"]));
    }

    #[test]
    fn custom_argv_is_rendered_with_unchanged_hardening() {
        let dir = bundle_dir("docbunker-bundle-argv-");
        let args = ["/bin/renderer-worker", "/host/marker", "4242"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        OciBundle::write_with_args(
            dir.path(),
            std::env::temp_dir().as_path(),
            &sample_config(),
            &args,
        )
        .unwrap();

        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(
            value["process"]["args"],
            json!(["/bin/renderer-worker", "/host/marker", "4242"])
        );
        assert_eq!(value["root"]["readonly"], true);
        assert_eq!(value["process"]["user"]["uid"], 65534);
        assert_eq!(value["process"]["capabilities"]["bounding"], json!([]));
        assert_eq!(value["process"]["env"], json!(["PATH=/bin", "HOME=/"]));
    }

    #[test]
    fn default_config_keeps_renderer_worker_as_init() {
        let dir = bundle_dir("docbunker-bundle-init-");
        OciBundle::write(dir.path(), std::env::temp_dir().as_path(), &sample_config()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["process"]["args"], json!(["/bin/renderer-worker"]));
    }

    #[test]
    fn renders_config_without_optional_resources() {
        let dir = bundle_dir("docbunker-bundle-test2-");
        let config = SandboxConfig {
            memory_limit_bytes: None,
            cpu_limit_millicpus: None,
            max_processes: None,
            ..SandboxConfig::default()
        };
        OciBundle::write(dir.path(), std::env::temp_dir().as_path(), &config).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value["linux"]["resources"]["memory"].is_null());
        assert!(value["linux"]["resources"]["cpu"].is_null());
        assert!(value["linux"]["resources"]["pids"].is_null());
    }

    #[test]
    fn tmp_mount_is_capped() {
        let dir = bundle_dir("docbunker-bundle-test3-");
        OciBundle::write(dir.path(), std::env::temp_dir().as_path(), &sample_config()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        let tmp = value["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["destination"] == "/tmp")
            .unwrap();
        assert!(tmp["options"]
            .as_array()
            .unwrap()
            .contains(&json!("size=262144k")));
        assert!(tmp["options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o.as_str().unwrap().contains("mode=0700")));
    }

    #[test]
    fn tmp_mount_uses_kibibytes_for_smaller_memory_limit() {
        let config = SandboxConfig {
            memory_limit_bytes: Some(128 * 1024 * 1024),
            ..sample_config()
        };
        let mounts = build_mounts(&config);
        let tmp = mounts.iter().find(|m| m["destination"] == "/tmp").unwrap();
        assert!(tmp["options"]
            .as_array()
            .unwrap()
            .contains(&json!("size=65536k")));
    }

    #[test]
    fn missing_rootfs_fails() {
        let dir = bundle_dir("docbunker-bundle-test4-");
        let result = OciBundle::write(
            dir.path(),
            Path::new("/definitely/not/a/rootfs"),
            &sample_config(),
        );
        assert!(matches!(result, Err(SandboxError::BackendUnsupported(_))));
    }

    #[test]
    fn guest_config_uses_rootfs_path_verbatim() {
        let dir = bundle_dir("docbunker-bundle-guest-");
        OciBundle::write_with_guest_rootfs(
            dir.path(),
            Path::new("/bundle/rootfs"),
            &sample_config(),
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["root"]["path"], json!("/bundle/rootfs"));
        assert_eq!(value["root"]["readonly"], true);
        assert_eq!(value["process"]["args"], json!(["/bin/renderer-worker"]));
    }

    #[test]
    fn refuses_to_overwrite_existing_config() {
        let dir = bundle_dir("docbunker-bundle-existing-");
        std::fs::write(dir.path().join("config.json"), "untrusted").unwrap();
        assert!(
            OciBundle::write(dir.path(), std::env::temp_dir().as_path(), &sample_config()).is_err()
        );
    }

    #[test]
    fn version_parsing() {
        assert_eq!(
            parse_runsc_version("runsc version 20250101.0 abcdef"),
            Some("20250101.0".to_string())
        );
        assert_eq!(
            parse_runsc_version("runsc version 1.2.3"),
            Some("1.2.3".to_string())
        );
        assert!(parse_runsc_version("something else").is_none());
        assert!(parse_runsc_version("").is_none());
    }
}
