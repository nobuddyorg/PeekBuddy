use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use super::{Action, Notifier, QUEUE_LEN, Tap, start};
use crate::event::tests::sample;
use crate::event::{Event, Kind, RootIdx};
use crate::name::WideStr;
use crate::rules::{Command, Rule};

const SECOND: u64 = 10_000_000;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Action>>>);

impl Notifier for Recorder {
    fn deliver(&mut self, action: Action) {
        if let Ok(mut list) = self.0.lock() {
            list.push(action);
        }
    }
}

/// Holds every delivery until the test sends on `gate`.
struct Gated(std::sync::mpsc::Receiver<()>);

impl Notifier for Gated {
    fn deliver(&mut self, _action: Action) {
        let _ = self.0.recv();
    }
}

fn event(ts_s: u64, kind: Kind) -> Event {
    Event {
        ts: ts_s * SECOND,
        kind,
        root: RootIdx(0),
        ..sample()
    }
}

fn rule(command: Option<Command>, toast: bool) -> Rule {
    Rule {
        name: String::from("Logs"),
        kinds: vec![String::from("created")],
        threshold: 2,
        window_s: 10,
        cooldown_s: 60,
        toast,
        command,
        ..Rule::default()
    }
}

fn feed(tap: &mut Tap, ts_s: u64, name: &str) {
    let name = units(name);
    tap.on_event(&event(ts_s, Kind::Created), WideStr::new(&name));
}

fn roots() -> Vec<Vec<u16>> {
    vec![units(r"C:\data")]
}

#[test]
fn a_firing_becomes_a_toast_and_a_command_with_the_event_filled_in() {
    let command = Command {
        argv: vec![
            String::from(r"C:\tools\h.exe"),
            String::from("%path%"),
            String::from("%kind%"),
        ],
        timeout_s: 5,
        confirmed: true,
    };
    let recorder = Recorder::default();
    let (mut tap, worker) = start(
        &[rule(Some(command), true)],
        &[String::from("w")],
        roots(),
        Box::new(recorder.clone()),
    )
    .unwrap();
    feed(&mut tap, 1, "a.log");
    feed(&mut tap, 2, "b c\\\"d.log");
    drop(tap);
    worker.join();
    let got = recorder.0.lock().unwrap().clone();
    assert_eq!(
        got,
        [
            Action::Toast {
                title: String::from("Logs"),
                body: String::from("2 matches"),
            },
            Action::Run {
                rule: String::from("Logs"),
                argv: vec![
                    units(r"C:\tools\h.exe"),
                    // One argument, whatever the name holds.
                    units("C:\\data\\b c\\\"d.log"),
                    units("created"),
                ],
                timeout_s: 5,
            },
        ]
    );
}

#[test]
fn the_cooldown_keeps_a_busy_stream_to_one_notification() {
    let recorder = Recorder::default();
    let (mut tap, worker) = start(
        &[rule(None, true)],
        &[],
        roots(),
        Box::new(recorder.clone()),
    )
    .unwrap();
    feed(&mut tap, 1, "a");
    for t in 2..30 {
        feed(&mut tap, t, "a");
    }
    // Inside the cooldown nothing else goes out.
    drop(tap);
    worker.join();
    assert_eq!(recorder.0.lock().unwrap().len(), 1);
}

#[test]
fn a_full_queue_drops_and_counts_instead_of_waiting() {
    let (gate, held) = channel();
    let rule = Rule {
        threshold: 1,
        cooldown_s: 0,
        ..rule(None, true)
    };
    let (mut tap, worker) = start(&[rule], &[], roots(), Box::new(Gated(held))).unwrap();
    // The worker takes one action and blocks; the queue holds QUEUE_LEN more.
    let total = QUEUE_LEN + 40;
    for t in 0..total {
        feed(&mut tap, u64::try_from(t).unwrap() + 1, "a");
    }
    let dropped = usize::try_from(tap.dropped()).unwrap();
    assert!(dropped >= 39, "{dropped}");
    assert!(dropped < total);
    drop(tap);
    for _ in 0..=total {
        let _ = gate.send(());
    }
    worker.join();
}

#[test]
fn a_bad_rule_is_reported_with_its_index() {
    let rules = [Rule {
        watches: vec![String::from("ghost")],
        ..rule(None, true)
    }];
    let why = start(&rules, &[], roots(), Box::new(Recorder::default()))
        .err()
        .unwrap_or_default();
    assert!(why.starts_with("rule[0]"), "{why}");
}
