//! Process metrics through documented Win32 APIs (#24).
//!
//! Every reader takes a process handle: [`current_process`] for this
//! process, or one from [`open_process`] (`PROCESS_QUERY_LIMITED_INFORMATION`)
//! so a harness can sample the engine from outside without waking it. PDH is
//! never used (`cargo xtask check-deps` rejects it): its "% Processor Time"
//! exceeds 100 % on multi-core machines and it matches processes by name.

pub mod process_list;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::*;

/// Memory of one process, in bytes.
///
/// Private Bytes and Working Set are different things and always reported
/// as separate fields: `private_bytes` is the commit charge (equal to
/// `PagefileUsage`), the working set includes shared pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    /// `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage`.
    pub private_bytes: u64,
    /// `WorkingSetSize`.
    pub working_set: u64,
    /// `PeakWorkingSetSize`.
    pub peak_working_set: u64,
    /// `PageFaultCount` (soft and hard faults).
    pub page_faults: u32,
}

/// CPU time of one process, in 100 ns units (`FILETIME` ticks).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimes {
    /// Creation time since 1601-01-01 UTC; the base of milestone offsets.
    pub creation: u64,
    /// Time spent in kernel mode.
    pub kernel: u64,
    /// Time spent in user mode.
    pub user: u64,
}

/// I/O since the process started (`IO_COUNTERS`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Io {
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Bytes of other I/O (neither read nor write).
    pub other_bytes: u64,
    /// Read operations.
    pub read_ops: u64,
    /// Write operations.
    pub write_ops: u64,
    /// Other operations.
    pub other_ops: u64,
}

/// GDI and USER objects held by one process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuiObjects {
    /// GDI objects.
    pub gdi: u32,
    /// USER objects.
    pub user: u32,
}

/// The process's own and the machine's native architecture
/// (`IMAGE_FILE_MACHINE_*` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arch {
    /// Architecture the process runs as.
    pub process: u16,
    /// Architecture of the machine.
    pub native: u16,
}

/// `IMAGE_FILE_MACHINE_UNKNOWN`: `IsWow64Process2` reports it for a process
/// that is not running under WOW64.
const MACHINE_UNKNOWN: u16 = 0;
/// `IMAGE_FILE_MACHINE_AMD64`.
const MACHINE_AMD64: u16 = 0x8664;
/// `IMAGE_FILE_MACHINE_ARM64`.
const MACHINE_ARM64: u16 = 0xAA64;

impl Arch {
    /// `true` when the process does not run natively (x64 on arm64, x86
    /// anywhere). Performance numbers of an emulated process are
    /// meaningless; diagnostics must say so.
    #[must_use]
    pub fn emulated(self) -> bool {
        self.process != MACHINE_UNKNOWN || !self.native_build()
    }

    /// `true` when this build's target matches the machine.
    fn native_build(self) -> bool {
        let built = if cfg!(target_arch = "aarch64") {
            MACHINE_ARM64
        } else {
            MACHINE_AMD64
        };
        self.native == built
    }

    /// Short name of an `IMAGE_FILE_MACHINE_*` value.
    #[must_use]
    pub fn name(machine: u16) -> &'static str {
        match machine {
            MACHINE_AMD64 => "x64",
            MACHINE_ARM64 => "arm64",
            0x014C => "x86",
            MACHINE_UNKNOWN => "native",
            _ => "other",
        }
    }
}

/// Joins the two halves of a `FILETIME`. The halves do not overlap, so `+`
/// equals `|` (and, unlike `|`, has no equivalent `^` mutant).
#[must_use]
pub fn filetime_ticks(low: u32, high: u32) -> u64 {
    (u64::from(high) << 32) + u64::from(low)
}

#[cfg(test)]
mod tests {
    use super::{Arch, MACHINE_AMD64, MACHINE_ARM64, MACHINE_UNKNOWN, filetime_ticks};

    fn this_machine() -> u16 {
        if cfg!(target_arch = "aarch64") {
            MACHINE_ARM64
        } else {
            MACHINE_AMD64
        }
    }

    #[test]
    fn a_native_process_is_not_emulated() {
        let arch = Arch {
            process: MACHINE_UNKNOWN,
            native: this_machine(),
        };
        assert!(!arch.emulated());
    }

    #[test]
    fn wow64_and_foreign_machines_are_emulated() {
        let wow64 = Arch {
            process: 0x014C,
            native: this_machine(),
        };
        assert!(wow64.emulated());
        let other = if this_machine() == MACHINE_ARM64 {
            MACHINE_AMD64
        } else {
            MACHINE_ARM64
        };
        let foreign = Arch {
            process: MACHINE_UNKNOWN,
            native: other,
        };
        assert!(foreign.emulated());
    }

    #[test]
    fn machine_names() {
        assert_eq!(Arch::name(MACHINE_AMD64), "x64");
        assert_eq!(Arch::name(MACHINE_ARM64), "arm64");
        assert_eq!(Arch::name(0x014C), "x86");
        assert_eq!(Arch::name(MACHINE_UNKNOWN), "native");
        assert_eq!(Arch::name(0x01C4), "other");
    }

    #[test]
    fn filetime_halves_join() {
        assert_eq!(
            filetime_ticks(0x89AB_CDEF, 0x0123_4567),
            0x0123_4567_89AB_CDEF
        );
        assert_eq!(filetime_ticks(1, 0), 1);
        assert_eq!(filetime_ticks(0, 1), 1 << 32);
    }
}
