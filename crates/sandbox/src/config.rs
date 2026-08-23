//! Per-session sandbox configuration.

use std::time::Duration;

use docbunker_renderer_api::limits;

/// Configuration for one sandbox session.
///
/// Values are enforced by the backend and, where possible, by the sandbox
/// itself (resource limits). `network_enabled` must remain `false` for the
/// renderer; it exists so the field is explicit and reviewable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// Maximum document size in bytes accepted in this session.
    pub max_document_size: usize,
    /// Maximum rendered page dimension.
    pub max_page_width: u32,
    pub max_page_height: u32,
    /// Maximum pixel buffer size in bytes.
    pub max_pixel_buffer: usize,
    /// Wall-clock deadline for open/info/render operations.
    pub operation_timeout: Duration,
    /// Deadline for destroying a session.
    pub shutdown_timeout: Duration,
    /// Memory limit for the sandbox (cgroup on Linux, VM RAM later).
    pub memory_limit_bytes: Option<u64>,
    /// CPU limit in millicpus (cgroup on Linux).
    pub cpu_limit_millicpus: Option<u64>,
    /// Maximum number of processes in the sandbox.
    pub max_processes: Option<u64>,
    /// Network connectivity for the sandbox. MUST stay false for the renderer.
    pub network_enabled: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_document_size: limits::MAX_DOCUMENT_SIZE,
            max_page_width: limits::MAX_PAGE_WIDTH,
            max_page_height: limits::MAX_PAGE_HEIGHT,
            max_pixel_buffer: limits::MAX_PIXEL_BUFFER,
            operation_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(10),
            memory_limit_bytes: Some(512 * 1024 * 1024),
            cpu_limit_millicpus: Some(1000),
            max_processes: Some(64),
            network_enabled: false,
        }
    }
}

impl SandboxConfig {
    /// Sanity check used by backends before honoring a config.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_document_size == 0
            || self.max_page_width == 0
            || self.max_page_height == 0
            || self.max_pixel_buffer == 0
        {
            return Err("zero limits are not allowed");
        }
        if self.network_enabled {
            return Err("network must be disabled for the renderer sandbox");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let config = SandboxConfig::default();
        assert!(config.validate().is_ok());
        assert!(!config.network_enabled);
        assert!(config.operation_timeout > Duration::ZERO);
    }

    #[test]
    fn rejects_network_enabled() {
        let config = SandboxConfig {
            network_enabled: true,
            ..SandboxConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
