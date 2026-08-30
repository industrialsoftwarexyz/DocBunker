//! Semantic version parsing and comparison for external tool binaries.
//!
//! DocBunker requires QEMU and `runsc` (gVisor) but does not enforce strict
//! minimum versions because distributions backport security fixes independently.
//! Instead, the host operator configures expected minimum versions and DocBunker
//! emits a warning (not a hard error) when the detected version is below the
//! floor. This prevents accidental regression while respecting the diverse
//! deployment landscape.

/// Parsed semantic version: `MAJOR.MINOR.PATCH`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemVer {
    /// Parse a dotted version string like `"8.2.2"` or `"20250101.0"`.
    ///
    /// Only the first three dot-separated components are used; trailing
    /// components (e.g. commit hash) are ignored. Missing components default
    /// to 0. Non-numeric components cause the parse to return `None`.
    pub fn parse(input: &str) -> Option<Self> {
        // Take only the first token (before any space) to handle suffixes like
        // "8.2.2 (qemu-8.2.2)".
        let version_str = input.split_whitespace().next()?;
        let mut parts = version_str.split('.');
        let major = parts.next()?.parse::<u32>().ok()?;
        let minor = match parts.next() {
            Some(s) => s.parse::<u32>().ok()?,
            None => 0,
        };
        let patch = match parts.next() {
            Some(s) => s.parse::<u32>().ok()?,
            None => 0,
        };
        Some(SemVer {
            major,
            minor,
            patch,
        })
    }

    /// Check whether `self` is below `minimum` and return a human-readable
    /// warning string if so, or `None` if `self >= minimum`.
    pub fn below_minimum_warning(&self, name: &str, minimum: &SemVer) -> Option<String> {
        if self < minimum {
            Some(format!(
                "{name} version {self} is below the recommended minimum {minimum}"
            ))
        } else {
            None
        }
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parse the QEMU version string from `qemu --version` output.
///
/// Expected format: `QEMU emulator version X.Y.Z (qemu-X.Y.Z)`.
pub fn parse_qemu_version(output: &str) -> Option<SemVer> {
    let version_part = output.lines().next()?;
    // Find "version " and take the token after it.
    let after_version = version_part.split("version ").nth(1)?;
    let token = after_version.split_whitespace().next()?;
    SemVer::parse(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parse_basic() {
        let v = SemVer::parse("8.2.2").unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 8,
                minor: 2,
                patch: 2
            }
        );
    }

    #[test]
    fn semver_parse_two_components() {
        let v = SemVer::parse("20250101.0").unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 20250101,
                minor: 0,
                patch: 0
            }
        );
    }

    #[test]
    fn semver_parse_with_suffix() {
        let v = SemVer::parse("8.2.2 (qemu-8.2.2)").unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 8,
                minor: 2,
                patch: 2
            }
        );
    }

    #[test]
    fn semver_parse_invalid() {
        assert!(SemVer::parse("").is_none());
        assert!(SemVer::parse("abc").is_none());
        assert!(SemVer::parse("1.x.3").is_none());
    }

    #[test]
    fn semver_ordering() {
        assert!(SemVer::parse("8.2.2").unwrap() < SemVer::parse("8.3.0").unwrap());
        assert!(SemVer::parse("8.2.2").unwrap() < SemVer::parse("9.0.0").unwrap());
        assert!(SemVer::parse("8.2.2").unwrap() == SemVer::parse("8.2.2").unwrap());
    }

    #[test]
    fn below_minimum_warning_works() {
        let v = SemVer {
            major: 7,
            minor: 0,
            patch: 0,
        };
        let min = SemVer {
            major: 8,
            minor: 0,
            patch: 0,
        };
        let warning = v.below_minimum_warning("QEMU", &min);
        assert!(warning.is_some());
        let warning = warning.unwrap();
        assert!(warning.contains("7.0.0"));
        assert!(warning.contains("8.0.0"));

        let v2 = SemVer {
            major: 8,
            minor: 1,
            patch: 0,
        };
        assert!(v2.below_minimum_warning("QEMU", &min).is_none());
    }

    #[test]
    fn parse_qemu_version_standard() {
        let output =
            "QEMU emulator version 8.2.2 (qemu-8.2.2)\nCopyright (c) 2003-2023 Fabrice Bellard";
        let v = parse_qemu_version(output).unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 8,
                minor: 2,
                patch: 2
            }
        );
    }

    #[test]
    fn parse_qemu_version_minimal() {
        let v = parse_qemu_version("QEMU emulator version 6.0.0").unwrap();
        assert_eq!(
            v,
            SemVer {
                major: 6,
                minor: 0,
                patch: 0
            }
        );
    }

    #[test]
    fn parse_qemu_version_invalid() {
        assert!(parse_qemu_version("").is_none());
        assert!(parse_qemu_version("not qemu").is_none());
    }
}
