//! Autostart at logon (#63): the decisions, without the registry.
//!
//! PeekBuddy starts at logon only when the user asks for it. The mechanism is
//! one value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which
//! Windows lists as a startup app, and Task Manager keeps its own switch for
//! that entry under `...\Explorer\StartupApproved\Run`. The rules:
//!
//! - The registry is the truth; settings store only the intent.
//! - If Task Manager disabled the entry, PeekBuddy reports it and never
//!   re-enables it at startup. Only an explicit toggle by the user may.
//! - If the value points to another exe (a portable copy moved) and the user
//!   wants autostart, the value is rewritten at startup.
//!
//! [`Autostart`] applies the rules to a [`Registry`]; the Windows side lives
//! in `peekbuddy_win::autostart`, tests use [`MemoryRegistry`].

/// The value name under the Run key.
pub const VALUE_NAME: &str = "PeekBuddy";

/// The 12 bytes Task Manager writes for an enabled entry: `02 00 ...`.
pub const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// What `StartupApproved` says about the entry.
///
/// The layout is not documented: the first byte is a flag where an even value
/// is "enabled" (`02`, `06`) and an odd one "disabled" (`03`, `07`; the rest
/// is the time of the change). Checked against a Task Manager toggle in the
/// owner run (#63).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// No value: Task Manager has not touched the entry.
    Absent,
    /// Enabled.
    Enabled,
    /// Disabled in Task Manager.
    Disabled,
}

impl Approval {
    /// Reads the binary value; an empty one counts as absent.
    #[must_use]
    pub fn parse(bytes: Option<&[u8]>) -> Self {
        match bytes.and_then(|b| b.first()) {
            None => Self::Absent,
            Some(flag) if flag & 1 == 0 => Self::Enabled,
            Some(_) => Self::Disabled,
        }
    }
}

/// The Run value for `exe`: the path in quotes, so a space in it cannot split
/// the command line.
#[must_use]
pub fn run_value(exe: &str) -> String {
    format!("\"{exe}\"")
}

/// What autostart looks like right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// No Run value.
    Off,
    /// Registered for this exe and enabled.
    On,
    /// Registered for another path (`current` is the value found).
    OtherPath {
        /// The value in the registry.
        current: String,
    },
    /// Registered, but disabled in Task Manager.
    DisabledInTaskManager,
}

/// The registry values autostart uses.
pub trait Registry {
    /// The error of a failed write.
    type Error;

    /// The Run value, if there is one.
    fn run_value(&self) -> Option<String>;
    /// Sets the Run value.
    ///
    /// # Errors
    ///
    /// When the registry refuses the write.
    fn set_run_value(&mut self, value: &str) -> Result<(), Self::Error>;
    /// Removes the Run value (no error if it is not there).
    ///
    /// # Errors
    ///
    /// When the registry refuses the delete.
    fn delete_run_value(&mut self) -> Result<(), Self::Error>;
    /// The `StartupApproved` bytes, if there are any.
    fn approved(&self) -> Option<Vec<u8>>;
    /// Sets the `StartupApproved` bytes.
    ///
    /// # Errors
    ///
    /// When the registry refuses the write.
    fn set_approved(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
}

/// The autostart rules over a registry, for one exe.
#[derive(Debug)]
pub struct Autostart<R> {
    registry: R,
    expected: String,
}

impl<R: Registry> Autostart<R> {
    /// Autostart for `exe` (an absolute path, unquoted) over `registry`.
    #[must_use]
    pub fn new(registry: R, exe: &str) -> Self {
        Self {
            registry,
            expected: run_value(exe),
        }
    }

    /// The registry, for inspection.
    #[must_use]
    pub const fn registry(&self) -> &R {
        &self.registry
    }

    /// What the registry says.
    #[must_use]
    pub fn status(&self) -> Status {
        let Some(current) = self.registry.run_value() else {
            return Status::Off;
        };
        if current != self.expected {
            return Status::OtherPath { current };
        }
        match Approval::parse(self.registry.approved().as_deref()) {
            Approval::Disabled => Status::DisabledInTaskManager,
            Approval::Absent | Approval::Enabled => Status::On,
        }
    }

    /// Brings the registry in line with the saved intent at engine start.
    ///
    /// With the intent off nothing is touched. With it on, a missing or stale
    /// Run value is written; the Task Manager switch is never changed.
    ///
    /// # Errors
    ///
    /// When the registry refuses the write.
    pub fn on_start(&mut self, intent: bool) -> Result<Status, R::Error> {
        if intent {
            match self.status() {
                Status::Off | Status::OtherPath { .. } => {
                    self.registry.set_run_value(&self.expected)?;
                }
                Status::On | Status::DisabledInTaskManager => {}
            }
        }
        Ok(self.status())
    }

    /// The user's explicit toggle. Turning it on also clears a Task Manager
    /// "disabled": the user asked for it.
    ///
    /// # Errors
    ///
    /// When the registry refuses the write.
    pub fn set(&mut self, enabled: bool) -> Result<Status, R::Error> {
        if enabled {
            self.registry.set_run_value(&self.expected)?;
            if Approval::parse(self.registry.approved().as_deref()) == Approval::Disabled {
                self.registry.set_approved(&APPROVED_ENABLED)?;
            }
        } else {
            self.registry.delete_run_value()?;
        }
        Ok(self.status())
    }
}

/// A registry in memory, for tests of the rules and of the callers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryRegistry {
    /// The Run value.
    pub run: Option<String>,
    /// The `StartupApproved` bytes.
    pub approved: Option<Vec<u8>>,
    /// How many writes were made.
    pub writes: usize,
}

impl Registry for MemoryRegistry {
    type Error = std::convert::Infallible;

    fn run_value(&self) -> Option<String> {
        self.run.clone()
    }

    fn set_run_value(&mut self, value: &str) -> Result<(), Self::Error> {
        self.run = Some(value.to_owned());
        self.writes += 1;
        Ok(())
    }

    fn delete_run_value(&mut self) -> Result<(), Self::Error> {
        self.run = None;
        self.writes += 1;
        Ok(())
    }

    fn approved(&self) -> Option<Vec<u8>> {
        self.approved.clone()
    }

    fn set_approved(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.approved = Some(bytes.to_vec());
        self.writes += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
