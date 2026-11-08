//! From a firing to an action (#81).
//!
//! The capture side must never wait for a toast or a program, so a firing
//! becomes an [`Action`] that goes into a bounded queue; one worker thread
//! hands the actions to a [`Notifier`] (the Win32 one lives in the engine, a
//! fake records in tests). A full queue drops the action and counts it
//! ([`Tap::dropped`]): a notification is never worth a stalled capture.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use super::eval::{Evaluator, Fired};
use super::subst::{Values, expand};
use super::{Rule, is_path_kind};
use crate::event::Event;
use crate::name::WideStr;

/// Actions waiting for the worker.
pub const QUEUE_LEN: usize = 64;

/// What a firing asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Tell the user: a toast, or a tray balloon when toasts are not
    /// available.
    Toast {
        /// The rule name.
        title: String,
        /// What happened.
        body: String,
    },
    /// Start a program (already confirmed by the user in the settings).
    Run {
        /// The rule name, for the log.
        rule: String,
        /// The arguments with the placeholders filled in, program first.
        argv: Vec<Vec<u16>>,
        /// Kill the program after this many seconds.
        timeout_s: u32,
    },
}

/// Delivers actions; runs on the worker thread only.
pub trait Notifier: Send {
    /// Shows or runs `action`. Failures are the notifier's to log.
    fn deliver(&mut self, action: Action);
}

/// What the tap needs to know about a rule beyond the evaluator.
#[derive(Debug, Clone)]
struct Meta {
    name: String,
    toast: bool,
    argv: Option<(Vec<String>, u32)>,
}

/// The evaluator on the capture side and the queue to the worker.
pub struct Tap {
    evaluator: Evaluator,
    meta: Vec<Meta>,
    /// The path of each root, by `RootIdx`.
    roots: Vec<Vec<u16>>,
    tx: SyncSender<Action>,
    dropped: Arc<AtomicU64>,
}

/// The worker that owns the notifier.
pub struct Worker {
    handle: Option<JoinHandle<()>>,
}

/// A tap and its worker.
///
/// # Errors
///
/// A rule that does not compile, or the OS error when the thread cannot start.
pub fn start(
    rules: &[Rule],
    watch_ids: &[String],
    roots: &[Vec<u16>],
    notifier: Box<dyn Notifier>,
) -> Result<(Tap, Worker), String> {
    let evaluator = Evaluator::compile(rules, watch_ids)?;
    let meta = rules
        .iter()
        .map(|rule| Meta {
            name: rule.name.clone(),
            toast: rule.toast,
            argv: rule
                .command
                .as_ref()
                .map(|command| (command.argv.clone(), command.timeout_s)),
        })
        .collect();
    let (tx, rx) = sync_channel(QUEUE_LEN);
    let handle = std::thread::Builder::new()
        .name(String::from("peekbuddy-rules"))
        .spawn(move || work(&rx, notifier))
        .map_err(|e| e.to_string())?;
    let tap = Tap {
        evaluator,
        meta,
        roots: roots.iter().map(|root| user_path(root)).collect(),
        tx,
        dropped: Arc::new(AtomicU64::new(0)),
    };
    Ok((
        tap,
        Worker {
            handle: Some(handle),
        },
    ))
}

/// A root as the user knows it: without the `\\?\` prefix of the long form
/// (`\\?\UNC\server\share` is `\\server\share`).
fn user_path(root: &[u16]) -> Vec<u16> {
    let long: Vec<u16> = r"\\?\".encode_utf16().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    if let Some(rest) = root.strip_prefix(unc.as_slice()) {
        let mut out: Vec<u16> = r"\\".encode_utf16().collect();
        out.extend_from_slice(rest);
        out
    } else if let Some(rest) = root.strip_prefix(long.as_slice()) {
        rest.to_vec()
    } else {
        root.to_vec()
    }
}

fn work(rx: &Receiver<Action>, mut notifier: Box<dyn Notifier>) {
    // `recv` ends when the tap is dropped.
    while let Ok(action) = rx.recv() {
        notifier.deliver(action);
    }
}

impl Worker {
    /// Waits until the queue is empty and the worker ended; call after
    /// dropping the tap.
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Tap {
    /// Actions that were dropped because the worker fell behind.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Evaluates one event of the stream; allocates only when a rule fires.
    pub fn on_event(&mut self, ev: &Event, name: WideStr<'_>) {
        let Self {
            evaluator,
            meta,
            roots,
            tx,
            dropped,
        } = self;
        evaluator.on_event(ev, name, |fired| {
            for action in actions(meta, roots, ev, name, &fired) {
                if tx.try_send(action).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }
}

fn full_path(root: &[u16], name: WideStr<'_>) -> Vec<u16> {
    let mut path = root.to_vec();
    if !name.is_empty() {
        if path.last() != Some(&u16::from(b'\\')) {
            path.push(u16::from(b'\\'));
        }
        path.extend_from_slice(name.units());
    }
    path
}

fn actions(
    meta: &[Meta],
    roots: &[Vec<u16>],
    ev: &Event,
    name: WideStr<'_>,
    fired: &Fired,
) -> Vec<Action> {
    let Some(meta) = meta.get(fired.rule) else {
        return Vec::new();
    };
    let root: &[u16] = roots.get(usize::from(ev.root.0)).map_or(&[], Vec::as_slice);
    let mut out = Vec::new();
    if meta.toast {
        let more = if fired.suppressed > 0 {
            format!(" ({} more while quiet)", fired.suppressed)
        } else {
            String::new()
        };
        out.push(Action::Toast {
            title: meta.name.clone(),
            body: format!(
                "{} {}{more}",
                fired.matches,
                if fired.matches == 1 {
                    "match"
                } else {
                    "matches"
                }
            ),
        });
    }
    if let Some((argv, timeout_s)) = &meta.argv {
        let path = full_path(root, name);
        let kind = if is_path_kind(ev.kind) {
            ev.kind.name()
        } else {
            ""
        };
        out.push(Action::Run {
            rule: meta.name.clone(),
            argv: expand(
                argv,
                &Values {
                    path: &path,
                    kind,
                    root,
                    ts: ev.ts,
                },
            ),
            timeout_s: *timeout_s,
        });
    }
    out
}

#[cfg(test)]
mod tests;
