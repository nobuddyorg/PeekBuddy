#![expect(
    clippy::indexing_slicing,
    reason = "the tests index the few firings they expect"
)]

use proptest::prelude::*;

use crate::event::tests::sample;
use crate::event::{Event, Kind, RootIdx};
use crate::name::WideStr;

use super::{Evaluator, Fired, SECOND};
use crate::rules::Rule;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn event(ts_s: u64, kind: Kind, root: u16) -> Event {
    Event {
        ts: ts_s * SECOND,
        kind,
        root: RootIdx(root),
        ..sample()
    }
}

fn rule(threshold: u32, window_s: u32, cooldown_s: u32) -> Rule {
    Rule {
        name: String::from("r"),
        threshold,
        window_s,
        cooldown_s,
        kinds: vec![String::from("created")],
        ..Rule::default()
    }
}

fn run(evaluator: &mut Evaluator, events: &[(Event, &str)]) -> Vec<Fired> {
    let mut out = Vec::new();
    for (ev, name) in events {
        let name = units(name);
        evaluator.on_event(ev, WideStr::new(&name), |f| out.push(f));
    }
    out
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn a_threshold_of_one_fires_on_every_match_outside_the_cooldown() {
    let mut evaluator = Evaluator::compile(&[rule(1, 10, 60)], &ids(&["w"])).unwrap();
    let fired = run(
        &mut evaluator,
        &[
            (event(100, Kind::Created, 0), "a"),
            (event(110, Kind::Created, 0), "b"),
            (event(159, Kind::Created, 0), "c"),
            (event(161, Kind::Created, 0), "d"),
        ],
    );
    // 100 fires; 110 and 159 are inside the cooldown; 161 fires with 2 more.
    assert_eq!(
        fired,
        [
            Fired {
                rule: 0,
                ts: 100 * SECOND,
                matches: 1,
                suppressed: 0
            },
            Fired {
                rule: 0,
                ts: 161 * SECOND,
                matches: 1,
                suppressed: 2
            },
        ]
    );
}

#[test]
fn n_matches_inside_the_window_fire_and_slow_ones_do_not() {
    let mut evaluator = Evaluator::compile(&[rule(3, 10, 0)], &ids(&["w"])).unwrap();
    let slow: Vec<_> = [0, 6, 12, 18, 24]
        .iter()
        .map(|&s| (event(s, Kind::Created, 0), "f"))
        .collect();
    // Never three within 10 s.
    assert_eq!(run(&mut evaluator, &slow), []);
    // 30, 35, 40 are three within 10 s.
    let fast = [
        (event(30, Kind::Created, 0), "f"),
        (event(35, Kind::Created, 0), "f"),
        (event(40, Kind::Created, 0), "f"),
    ];
    let fired = run(&mut evaluator, &fast);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].ts, 40 * SECOND);
    assert_eq!(fired[0].matches, 3);
}

#[test]
fn a_burst_that_fired_starts_counting_again_from_zero() {
    let mut evaluator = Evaluator::compile(&[rule(2, 10, 0)], &ids(&["w"])).unwrap();
    let burst: Vec<_> = (0..5).map(|s| (event(s, Kind::Created, 0), "f")).collect();
    // 0+1 fire, 2+3 fire, 4 waits.
    let fired = run(&mut evaluator, &burst);
    assert_eq!(
        fired.iter().map(|f| f.ts / SECOND).collect::<Vec<_>>(),
        [1, 3]
    );
}

#[test]
fn kinds_roots_and_globs_all_have_to_match() {
    let mut candidate = rule(1, 10, 0);
    candidate.watches = ids(&["b"]);
    candidate.glob = String::from("*.log");
    candidate.kinds = vec![String::from("created"), String::from("deleted")];
    let mut evaluator = Evaluator::compile(&[candidate], &ids(&["a", "b"])).unwrap();
    let fired = run(
        &mut evaluator,
        &[
            (event(1, Kind::Created, 1), "x.log"),
            (event(2, Kind::Modified, 1), "x.log"),
            (event(3, Kind::Created, 0), "x.log"),
            (event(4, Kind::Created, 1), "x.txt"),
            (event(5, Kind::Deleted, 1), "dir\\y.log"),
            (event(6, Kind::Created, 7), "x.log"),
        ],
    );
    assert_eq!(
        fired.iter().map(|f| f.ts / SECOND).collect::<Vec<_>>(),
        [1, 5]
    );
}

#[test]
fn a_disabled_rule_keeps_its_place_and_never_fires() {
    let mut off = rule(1, 10, 0);
    off.enabled = false;
    let mut evaluator = Evaluator::compile(&[off, rule(1, 10, 0)], &ids(&["w"])).unwrap();
    assert_eq!(evaluator.len(), 2);
    let fired = run(&mut evaluator, &[(event(1, Kind::Created, 0), "a")]);
    assert_eq!(fired.iter().map(|f| f.rule).collect::<Vec<_>>(), [1]);
}

#[test]
fn several_rules_fire_independently_for_one_event() {
    let mut evaluator =
        Evaluator::compile(&[rule(1, 10, 0), rule(1, 10, 0)], &ids(&["w"])).unwrap();
    let fired = run(&mut evaluator, &[(event(1, Kind::Created, 0), "a")]);
    assert_eq!(fired.iter().map(|f| f.rule).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn compile_names_the_rule_that_is_wrong() {
    let mut bad = rule(1, 10, 0);
    bad.watches = ids(&["nope"]);
    let error = Evaluator::compile(&[rule(1, 10, 0), bad], &ids(&["w"])).unwrap_err();
    assert!(error.starts_with("rule[1]:"), "{error}");
    let mut bad = rule(1, 10, 0);
    bad.kinds = vec![String::from("sneezed")];
    assert!(Evaluator::compile(&[bad], &[]).is_err());
}

#[test]
fn a_clock_that_goes_back_does_not_wrap() {
    let mut evaluator = Evaluator::compile(&[rule(2, 10, 0)], &ids(&["w"])).unwrap();
    let fired = run(
        &mut evaluator,
        &[
            (event(100, Kind::Created, 0), "a"),
            (event(50, Kind::Created, 0), "a"),
        ],
    );
    // 100 then 50: a distance of zero, inside the window.
    assert_eq!(fired.len(), 1);
}

/// The slow, obvious version: a list of match times since the last firing.
fn reference(threshold: usize, window: u64, cooldown: u64, matches: &[u64]) -> Vec<Fired> {
    let mut out = Vec::new();
    let mut pending: Vec<u64> = Vec::new();
    let mut quiet_until = 0;
    let mut suppressed = 0;
    for &ts in matches {
        if ts < quiet_until {
            suppressed += 1;
            continue;
        }
        pending.push(ts);
        if pending.len() >= threshold {
            let oldest = pending[pending.len() - threshold];
            if ts.saturating_sub(oldest) <= window {
                out.push(Fired {
                    rule: 0,
                    ts,
                    matches: u32::try_from(threshold).unwrap(),
                    suppressed,
                });
                suppressed = 0;
                pending.clear();
                quiet_until = ts + cooldown;
            }
        }
    }
    out
}

proptest! {
    /// The ring counter equals the obvious model for any rule and any
    /// sequence of matches and non-matches (the clock only goes forward).
    #[test]
    fn the_counter_matches_the_reference_model(
        threshold in 1_u32..6,
        window_s in 1_u32..30,
        cooldown_s in 0_u32..40,
        steps in proptest::collection::vec((0_u64..8, any::<bool>()), 0..120),
    ) {
        let mut evaluator =
            Evaluator::compile(&[rule(threshold, window_s, cooldown_s)], &ids(&["w"])).unwrap();
        let mut clock = 1_000_u64;
        let mut events = Vec::new();
        let mut matched = Vec::new();
        for (dt, hit) in steps {
            clock += dt;
            let kind = if hit { Kind::Created } else { Kind::Modified };
            events.push((event(clock, kind, 0), "f"));
            if hit {
                matched.push(clock * SECOND);
            }
        }
        let got = run(&mut evaluator, &events);
        let want = reference(
            threshold as usize,
            u64::from(window_s) * SECOND,
            u64::from(cooldown_s) * SECOND,
            &matched,
        );
        prop_assert_eq!(got, want);
    }
}
