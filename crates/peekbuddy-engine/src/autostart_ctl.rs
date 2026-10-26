//! Autostart as the engine runs it (#63): the saved intent applied to the
//! registry at start and when settings change.
//!
//! The rules are in [`peekbuddy_core::autostart`]. Test engines (an explicit
//! data directory, headless) get no controller and never touch the registry.

use std::fmt::Display;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use peekbuddy_core::autostart::{Autostart, Registry, Status};

/// The text of a status for messages and diagnostics.
#[must_use]
pub fn describe(status: &Status) -> String {
    match status {
        Status::Off => String::from("autostart off"),
        Status::On => String::from("autostart on"),
        Status::OtherPath { current } => {
            format!("autostart points to another program: {current}")
        }
        Status::DisabledInTaskManager => String::from("autostart disabled in Task Manager"),
    }
}

/// The intent and the registry behind it.
pub struct AutostartCtl<R: Registry> {
    auto: Mutex<Autostart<R>>,
    intent: AtomicBool,
}

impl<R: Registry> std::fmt::Debug for AutostartCtl<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutostartCtl")
            .field("intent", &self.intent.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl<R: Registry> AutostartCtl<R>
where
    R::Error: Display,
{
    /// Applies the saved `intent` at engine start (a missing or stale value is
    /// written; Task Manager's switch is left alone) and keeps it. The result
    /// is what the registry says afterwards, or why a write failed.
    pub fn start(auto: Autostart<R>, intent: bool) -> (Self, Result<Status, String>) {
        let mut auto = auto;
        let result = auto.on_start(intent).map_err(|error| error.to_string());
        (
            Self {
                auto: Mutex::new(auto),
                intent: AtomicBool::new(intent),
            },
            result,
        )
    }

    /// What the registry says now.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        self.auto.lock().ok().map(|auto| auto.status())
    }

    /// Settings were saved with `intent`. A change is an explicit toggle by
    /// the user and may also clear Task Manager's "disabled"; an unchanged
    /// intent only repairs a stale value.
    ///
    /// # Errors
    ///
    /// A message when the registry refuses a write.
    pub fn apply(&self, intent: bool) -> Result<Status, String> {
        let mut auto = self
            .auto
            .lock()
            .map_err(|_| String::from("autostart control is unavailable"))?;
        let before = self.intent.swap(intent, Ordering::AcqRel);
        let result = if before == intent {
            auto.on_start(intent)
        } else {
            auto.set(intent)
        };
        result.map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::autostart::{MemoryRegistry, run_value};

    use super::*;

    const EXE: &str = "C:/tools/peekbuddy.exe";

    fn disabled() -> Vec<u8> {
        vec![3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]
    }

    #[test]
    fn the_start_repairs_a_stale_value_and_respects_task_manager() {
        let registry = MemoryRegistry {
            run: Some(String::from("\"D:/old.exe\"")),
            approved: Some(disabled()),
            writes: 0,
        };
        let (ctl, started) = AutostartCtl::start(Autostart::new(registry, EXE), true);
        assert_eq!(started, Ok(Status::DisabledInTaskManager));
        assert_eq!(
            describe(&Status::DisabledInTaskManager),
            "autostart disabled in Task Manager"
        );
        // Saving the same intent again does not re-enable it either.
        assert_eq!(ctl.apply(true), Ok(Status::DisabledInTaskManager));
        // Turning it off and on again is the user's explicit toggle.
        assert_eq!(ctl.apply(false), Ok(Status::Off));
        assert_eq!(ctl.apply(true), Ok(Status::On));
        assert_eq!(ctl.status(), Some(Status::On));
    }

    #[test]
    fn a_changed_intent_writes_and_an_off_start_touches_nothing() {
        let (ctl, started) =
            AutostartCtl::start(Autostart::new(MemoryRegistry::default(), EXE), false);
        assert_eq!(started, Ok(Status::Off));
        assert_eq!(ctl.apply(true), Ok(Status::On));
        let auto = ctl.auto.lock().unwrap();
        assert_eq!(
            auto.registry().run.as_deref(),
            Some(run_value(EXE).as_str())
        );
        assert_eq!(auto.registry().approved, None);
    }
}
