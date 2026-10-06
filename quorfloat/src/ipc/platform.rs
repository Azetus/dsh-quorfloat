//! Platform and architecture names in the host's vocabulary.
//!
//! Rust and Node name the same machine differently: `std::env::consts::OS` is
//! `macos` where Node's `process.platform` is `darwin`, and `aarch64` where Node
//! says `arm64`. The `hello` frame reports these facts, and the host is a Node
//! process that will compare them with its own names — most obviously the binary
//! resolver, which already looks for `dsh-quorfloat-${process.platform}-${process.arch}`.
//!
//! So the wire always carries the **host's** spelling, never Rust's. Reporting
//! `macos` would not be wrong exactly, but it would be useless: nothing on the
//! other side branches on that string, and a platform check written against it
//! would silently never match.

/// The current platform, spelled the way the host spells it.
///
/// @returns `process.platform`'s value for this machine, or Rust's `OS` string
/// when it has no Node equivalent.
#[must_use]
pub fn host_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// The current architecture, spelled the way the host spells it.
///
/// @returns `process.arch`'s value for this machine, or Rust's `ARCH` string
/// when it has no Node equivalent.
#[must_use]
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_machine_reports_node_spelling() {
        // Pinned against Node's values for the platforms this project targets, so
        // a Rust upgrade that renames a constant fails here rather than silently
        // reporting a platform the host cannot match.
        let platform = host_platform();
        let arch = host_arch();
        if cfg!(target_os = "macos") {
            assert_eq!(platform, "darwin");
        }
        if cfg!(target_os = "windows") {
            assert_eq!(platform, "win32");
        }
        if cfg!(target_os = "linux") {
            assert_eq!(platform, "linux");
        }
        if cfg!(target_arch = "aarch64") {
            assert_eq!(arch, "arm64");
        }
        if cfg!(target_arch = "x86_64") {
            assert_eq!(arch, "x64");
        }
    }

    #[test]
    fn the_names_match_the_package_naming_scheme() {
        // `dsh-quorfloat-<platform>-<arch>` is what the host's resolver looks for,
        // so these two strings have to compose into a real directory name.
        let package = format!("dsh-quorfloat-{}-{}", host_platform(), host_arch());
        assert!(!package.contains("macos"));
        assert!(!package.contains("aarch64"));
        assert_eq!(package.split('-').count(), 4, "got {package}");
    }
}
