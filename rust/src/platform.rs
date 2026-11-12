//! Cross-platform shims for `std::os::windows` extension traits.
//!
//! The manager's core logic calls `.creation_flags(...)`, `.file_size()` and
//! friends on `std` types. Those methods only exist on Windows. Rather than
//! scattering `#[cfg]` through every call site, this module re-exports traits
//! with the SAME method names: on Windows they forward to the real API, on
//! other targets they are documented no-ops or portable equivalents.
//!
//! Consequence: a file only needs `use crate::platform::CommandExt;` instead of
//! `use std::os::windows::process::CommandExt;` and its body stays unchanged.

use std::fs::OpenOptions;
use std::process::Command;

/// Mirrors `std::os::windows::process::CommandExt`.
///
/// `creation_flags` is used exclusively to pass `CREATE_NO_WINDOW`
/// (0x0800_0000) so child processes never flash a console. Linux has no such
/// concept — a GUI-launched child has no terminal attached anyway — so the
/// non-Windows implementation ignores the flags. Callers keep compiling and
/// behaving identically from the user's point of view.
pub trait CommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

impl CommandExt for Command {
    #[cfg(windows)]
    fn creation_flags(&mut self, flags: u32) -> &mut Self {
        std::os::windows::process::CommandExt::creation_flags(self, flags)
    }
    #[cfg(not(windows))]
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        self
    }
}

/// Mirrors `std::os::windows::fs::MetadataExt`, limited to the three methods
/// this project uses. Portability notes:
///
/// * `file_size` -> `Metadata::len()`.
/// * `last_write_time` is only ever compared for *change* (a stamp), never
///   interpreted as a date, so Unix nanosecond mtime is a faithful stand-in
///   for the Windows `FILETIME` (100 ns units since 1601).
/// * `file_attributes` -> 0. The one bit the manager tests is
///   `FILE_ATTRIBUTE_REPARSE_POINT` (0x400), which is a Windows-only redirect
///   concept; on Unix the equivalent check is `FileType::is_symlink()`, which
///   `core::is_link` performs *in addition* to this mask, so no check is lost.
pub trait MetadataExt {
    fn file_size(&self) -> u64;
    fn last_write_time(&self) -> u64;
    fn file_attributes(&self) -> u32;
}

impl MetadataExt for std::fs::Metadata {
    #[cfg(windows)]
    fn file_size(&self) -> u64 {
        std::os::windows::fs::MetadataExt::file_size(self)
    }
    #[cfg(not(windows))]
    fn file_size(&self) -> u64 {
        self.len()
    }

    #[cfg(windows)]
    fn last_write_time(&self) -> u64 {
        std::os::windows::fs::MetadataExt::last_write_time(self)
    }
    #[cfg(not(windows))]
    fn last_write_time(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        // Seconds and nanoseconds combined into one comparable stamp.
        self.mtime().unsigned_abs() * 1_000_000_000 + self.mtime_nsec().unsigned_abs()
    }

    #[cfg(windows)]
    fn file_attributes(&self) -> u32 {
        std::os::windows::fs::MetadataExt::file_attributes(self)
    }
    #[cfg(not(windows))]
    fn file_attributes(&self) -> u32 {
        0
    }
}

/// Mirrors the `std::os::windows::fs::OpenOptionsExt` methods used by the delta
/// runtime cleanup, which opens a component with `GENERIC_READ | DELETE` and
/// *no write sharing* so the file cannot change between identity verification
/// and removal.
///
/// Linux has no share-mode concept, and an advisory lock does not stop other
/// processes, so the non-Windows implementation records nothing. The delta
/// cleanup remains correct because it re-verifies the component hash after
/// opening and before unlinking (the same second check the Windows path makes),
/// and `core::no_links` rejects links.
pub trait OpenOptionsExt {
    fn access_mode(&mut self, mode: u32) -> &mut Self;
    fn share_mode(&mut self, share: u32) -> &mut Self;
}

impl OpenOptionsExt for OpenOptions {
    #[cfg(windows)]
    fn access_mode(&mut self, mode: u32) -> &mut Self {
        std::os::windows::fs::OpenOptionsExt::access_mode(self, mode)
    }
    #[cfg(not(windows))]
    fn access_mode(&mut self, _mode: u32) -> &mut Self {
        self
    }

    #[cfg(windows)]
    fn share_mode(&mut self, share: u32) -> &mut Self {
        std::os::windows::fs::OpenOptionsExt::share_mode(self, share)
    }
    #[cfg(not(windows))]
    fn share_mode(&mut self, _share: u32) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_creation_flags_is_chainable_on_every_platform() {
        // Must compile and return the receiver so builder chains keep working.
        let mut command = Command::new("true");
        let chained = command.creation_flags(0x0800_0000);
        let _ = chained;
        let _ = command.creation_flags(0);
    }

    #[test]
    fn metadata_shim_reports_portable_values() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("stamp.bin");
        std::fs::write(&path, b"0123456789").expect("write");
        let meta = std::fs::metadata(&path).expect("metadata");
        assert_eq!(meta.file_size(), 10);
        #[cfg(not(windows))]
        assert_eq!(meta.file_attributes(), 0);
    }

    #[test]
    fn open_options_shim_keeps_default_open_semantics() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("component.dll");
        std::fs::write(&path, b"payload").expect("write");
        let file = OpenOptions::new()
            .read(true)
            .access_mode(0x8001_0000)
            .share_mode(0x5)
            .open(&path)
            .expect("open");
        assert_eq!(file.metadata().expect("metadata").len(), 7);
    }
}
