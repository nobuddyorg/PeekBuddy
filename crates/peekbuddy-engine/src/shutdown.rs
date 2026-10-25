//! The one shutdown path.
//!
//! Quit, `WM_ENDSESSION`, a pipe shutdown and Ctrl+C in headless mode all end
//! in [`Shutdown::run`]. Components register a step for the stage they own
//! (capture, writer, store, tray) when they come to exist; the stages always
//! run in the same order whatever the registration order.

/// Why the engine is shutting down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The tray menu, the pipe or `WM_CLOSE`.
    Quit,
    /// `WM_ENDSESSION`: logoff or system shutdown.
    EndSession,
}

/// Shutdown stages, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// `CancelIoEx` on every watch and join the readers.
    StopCapture,
    /// Move what is left in the ring to the writer.
    DrainRing,
    /// Commit the last batch.
    FinalCommit,
    /// Persist the per-root checkpoints.
    PersistCheckpoints,
    /// Close the database.
    CloseDatabase,
    /// Remove the tray icon.
    RemoveTray,
    /// Unregister the ETW provider: the last thing before the process ends,
    /// also on `WM_ENDSESSION`, where the OS may end the process right after
    /// the handler returns.
    ReleaseProvider,
}

type Step = Box<dyn FnMut(Reason)>;

/// Registered steps; runs each exactly once.
#[derive(Default)]
pub struct Shutdown {
    steps: Vec<(Stage, Step)>,
    done: bool,
}

impl std::fmt::Debug for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shutdown")
            .field("steps", &self.steps.len())
            .field("done", &self.done)
            .finish()
    }
}

impl Shutdown {
    /// Creates an empty shutdown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `step` for `stage`. Steps of one stage run in registration
    /// order.
    pub fn register(&mut self, stage: Stage, step: impl FnMut(Reason) + 'static) {
        self.steps.push((stage, Box::new(step)));
    }

    /// Whether [`Shutdown::run`] already ran.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.done
    }

    /// Runs every step in stage order. A second call does nothing, so
    /// `WM_CLOSE` after `WM_ENDSESSION` (or the reverse) cannot flush twice.
    pub fn run(&mut self, reason: Reason) {
        if self.done {
            return;
        }
        self.done = true;
        // Stable sort keeps registration order inside a stage.
        self.steps.sort_by_key(|(stage, _)| *stage);
        for (_, step) in &mut self.steps {
            step(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{Reason, Shutdown, Stage};

    fn recorder() -> (Rc<RefCell<Vec<&'static str>>>, Shutdown) {
        (Rc::default(), Shutdown::new())
    }

    #[test]
    fn stages_run_in_order_whatever_the_registration_order() {
        let (log, mut shutdown) = recorder();
        for (stage, name) in [
            (Stage::ReleaseProvider, "provider"),
            (Stage::RemoveTray, "tray"),
            (Stage::CloseDatabase, "close"),
            (Stage::StopCapture, "stop"),
            (Stage::FinalCommit, "commit"),
            (Stage::PersistCheckpoints, "checkpoints"),
            (Stage::DrainRing, "drain"),
        ] {
            let log = Rc::clone(&log);
            shutdown.register(stage, move |_| log.borrow_mut().push(name));
        }
        shutdown.run(Reason::Quit);
        assert_eq!(
            *log.borrow(),
            [
                "stop",
                "drain",
                "commit",
                "checkpoints",
                "close",
                "tray",
                "provider"
            ]
        );
    }

    #[test]
    fn a_stage_keeps_registration_order() {
        let (log, mut shutdown) = recorder();
        for name in ["a", "b", "c"] {
            let log = Rc::clone(&log);
            shutdown.register(Stage::FinalCommit, move |_| log.borrow_mut().push(name));
        }
        shutdown.run(Reason::Quit);
        assert_eq!(*log.borrow(), ["a", "b", "c"]);
    }

    #[test]
    fn running_twice_runs_each_step_once_and_passes_the_reason() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut shutdown = Shutdown::new();
        let sink = Rc::clone(&seen);
        shutdown.register(Stage::StopCapture, move |reason| {
            sink.borrow_mut().push(reason);
        });
        assert!(!shutdown.is_done());
        shutdown.run(Reason::EndSession);
        shutdown.run(Reason::Quit);
        assert!(shutdown.is_done());
        assert_eq!(*seen.borrow(), [Reason::EndSession]);
    }
}
