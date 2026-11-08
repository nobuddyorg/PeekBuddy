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

fn command() -> Command {
    Command {
        argv: vec![String::from(r"C:	ools\h.exe")],
        confirmed: true,
        ..Command::default()
    }
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
        &roots(),
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
        &roots(),
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
        ..rule(Some(command()), false)
    };
    let (mut tap, worker) = start(&[rule], &[], &roots(), Box::new(Gated(held))).unwrap();
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
    let why = start(&rules, &[], &roots(), Box::new(Recorder::default()))
        .err()
        .unwrap_or_default();
    assert!(why.starts_with("rule[0]"), "{why}");
}

#[test]
fn the_user_sees_the_path_without_the_long_prefix() {
    for (root, shown) in [
        (r"\\?\C:\data", r"C:\data"),
        (r"\\?\UNC\srv\share\d", r"\\srv\share\d"),
        (r"D:\plain", r"D:\plain"),
    ] {
        assert_eq!(super::user_path(&units(root)), units(shown), "{root}");
    }
}

#[test]
fn toasts_are_capped_per_minute_over_all_rules_and_counted() {
    // Every event fires its own rule at once: ten rules, ten toasts wanted.
    let rules: Vec<Rule> = (0..10)
        .map(|i| Rule {
            name: format!("r{i}"),
            threshold: 1,
            cooldown_s: 0,
            ..rule(None, true)
        })
        .collect();
    let recorder = Recorder::default();
    let (mut tap, worker) = start(&rules, &[], &roots(), Box::new(recorder.clone())).unwrap();
    feed(&mut tap, 1, "a");
    assert_eq!(tap.capped(), 4);
    // A minute later the next ones go out again.
    feed(&mut tap, 70, "a");
    assert_eq!(tap.capped(), 8);
    drop(tap);
    worker.join();
    assert_eq!(
        recorder.0.lock().unwrap().len(),
        2 * super::TOASTS_PER_MINUTE
    );
}

/// A rule whose command writes into the watched folder: every run produces
/// the next event. The cooldown lets one run per cooldown through, so a
/// day of this is a few hundred runs and not a runaway.
#[test]
fn a_command_that_triggers_its_own_rule_is_throttled_by_the_cooldown() {
    let command = Command {
        argv: vec![String::from(r"C:\tools\writes.exe")],
        confirmed: true,
        ..Command::default()
    };
    let rule = Rule {
        threshold: 1,
        cooldown_s: 60,
        ..rule(Some(command), false)
    };
    let recorder = Recorder::default();
    let (mut tap, worker) = start(&[rule], &[], &roots(), Box::new(recorder.clone())).unwrap();
    // Each run's output file arrives 100 ms after the run starts, for ten
    // simulated minutes: 6,000 events.
    let mut ts = 0_u64;
    for _ in 0..6_000 {
        ts += SECOND / 10;
        let name = units("out.log");
        tap.on_event(&event(0, Kind::Created).with_ts(ts), WideStr::new(&name));
    }
    drop(tap);
    worker.join();
    let runs = recorder.0.lock().unwrap().len();
    assert!((10..=11).contains(&runs), "{runs}");
}

trait WithTs {
    fn with_ts(self, ts: u64) -> Self;
}

impl WithTs for Event {
    fn with_ts(mut self, ts: u64) -> Self {
        self.ts = ts;
        self
    }
}

#[test]
fn the_toast_after_a_quiet_spell_says_how_many_came_meanwhile() {
    let rule = Rule {
        threshold: 1,
        cooldown_s: 10,
        ..rule(None, true)
    };
    let recorder = Recorder::default();
    let (mut tap, worker) = start(&[rule], &[], &roots(), Box::new(recorder.clone())).unwrap();
    for t in [1, 2, 3, 20] {
        feed(&mut tap, t, "a");
    }
    drop(tap);
    worker.join();
    let bodies: Vec<String> = recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|action| match action {
            Action::Toast { body, .. } => Some(body.clone()),
            Action::Run { .. } => None,
        })
        .collect();
    assert_eq!(bodies, ["1 match", "1 match (2 more while quiet)"]);
}

#[test]
fn the_seventh_toast_is_allowed_exactly_a_minute_after_the_first() {
    let rules: Vec<Rule> = (0..=super::TOASTS_PER_MINUTE)
        .map(|i| Rule {
            name: format!("r{i}"),
            threshold: 1,
            cooldown_s: 0,
            ..rule(None, true)
        })
        .collect();
    let recorder = Recorder::default();
    let (mut tap, worker) = start(&rules, &[], &roots(), Box::new(recorder.clone())).unwrap();
    // All seven fire at 1 s: six go out. At 61 s the first is exactly a
    // minute old and its slot is free again.
    feed(&mut tap, 1, "a");
    assert_eq!(tap.capped(), 1);
    feed(&mut tap, 61, "a");
    assert_eq!(tap.capped(), 2, "six out of seven again, not zero");
    drop(tap);
    worker.join();
}
