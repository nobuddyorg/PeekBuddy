//! The Windows side of rule actions (#81): a tray balloon for a toast and a
//! kill-on-timeout child for a command.
//!
//! A rule runs one command at a time: a firing while the previous run of the
//! same rule is still going is logged and skipped, so a rule that fires on
//! its own command's output cannot start a fork bomb.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use peekbuddy_core::rules::notify::{Action, Notifier};
use peekbuddy_core::rules::subst::command_line;
use peekbuddy_core::settings::Settings;
use peekbuddy_win::spawn::spawn;
use peekbuddy_win::tray::Tray;
use peekbuddy_win::window::WindowHandle;

use crate::capture_host::WatchSpec;

/// The engine's hidden window, which owns the tray icon; the capture starts
/// before the window exists, so the notifier looks it up when it needs it.
static WINDOW: OnceLock<WindowHandle> = OnceLock::new();

/// Tells the notifier which window owns the tray icon.
pub fn bind_window(window: WindowHandle) {
    let _ = WINDOW.set(window);
}

/// The tap for the rules of `settings` and its worker thread, or `None`
/// without enabled rules. A rule that does not compile (the validator
/// refused it before) is logged and nothing is evaluated.
#[must_use]
pub fn start(
    settings: &Settings,
    watches: &[WatchSpec],
) -> Option<peekbuddy_core::rules::notify::Tap> {
    if !settings.rules.iter().any(|rule| rule.enabled) {
        return None;
    }
    let ids: Vec<String> = settings.watches.iter().map(|w| w.id.clone()).collect();
    let roots: Vec<Vec<u16>> = watches.iter().map(|watch| watch.plain.clone()).collect();
    match peekbuddy_core::rules::notify::start(
        &settings.rules,
        &ids,
        &roots,
        Box::new(WinNotifier::new()),
    ) {
        // The worker ends when the tap is dropped with the sink.
        Ok((tap, _worker)) => Some(tap),
        Err(why) => {
            crate::instrument::log(&format!("rules: not started: {why}"));
            None
        }
    }
}

/// Delivers rule actions on Windows.
pub struct WinNotifier {
    running: Arc<Mutex<HashSet<String>>>,
}

impl WinNotifier {
    /// A notifier whose balloons go through the tray icon of the engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl Default for WinNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier for WinNotifier {
    fn deliver(&mut self, action: Action) {
        match action {
            Action::Toast { title, body } => {
                let Some(window) = WINDOW.get() else {
                    return;
                };
                if let Err(error) = Tray::new(*window).balloon(&title, &body) {
                    crate::instrument::log(&format!("rules: the balloon failed: {error}"));
                }
            }
            Action::Run {
                rule,
                argv,
                timeout_s,
            } => self.run(rule, &argv, timeout_s),
        }
    }
}

impl WinNotifier {
    fn run(&self, rule: String, argv: &[Vec<u16>], timeout_s: u32) {
        let Some(program) = argv.first() else {
            return;
        };
        let line = command_line(argv);
        let fresh = self
            .running
            .lock()
            .is_ok_and(|mut running| running.insert(rule.clone()));
        if !fresh {
            crate::instrument::log(&format!("rules: `{rule}` is still running, skipped"));
            return;
        }
        let child = match spawn(program, &line) {
            Ok(child) => child,
            Err(error) => {
                crate::instrument::log(&format!("rules: `{rule}` did not start: {error}"));
                self.finish(&rule);
                return;
            }
        };
        let running = Arc::clone(&self.running);
        let name = rule.clone();
        let spawned = std::thread::Builder::new()
            .name(String::from("peekbuddy-rule-run"))
            .spawn(move || {
                let wait = u32::try_from(Duration::from_secs(u64::from(timeout_s)).as_millis())
                    .unwrap_or(u32::MAX);
                match child.wait(wait) {
                    Ok(Some(code)) => {
                        crate::instrument::log(&format!("rules: `{rule}` ended with {code}"));
                    }
                    Ok(None) => {
                        crate::instrument::log(&format!(
                            "rules: `{rule}` ran over {timeout_s} s and was killed"
                        ));
                        child.kill();
                    }
                    Err(error) => {
                        crate::instrument::log(&format!("rules: waiting for `{rule}`: {error}"));
                        child.kill();
                    }
                }
                if let Ok(mut running) = running.lock() {
                    running.remove(&rule);
                }
            });
        if spawned.is_err() {
            // The closure, and the child with it, are gone: the job kills it.
            self.finish(&name);
        }
    }

    fn finish(&self, rule: &str) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(rule);
        }
    }
}
