//! `cargo xtask churn`: runs a churn profile against a directory and writes
//! the ground-truth ledger (#25). Windows only; see `docs/how-to/churn.md`.

use std::fmt::Write as _;
use std::path::PathBuf;

use peekbuddy_churn::exec::Outcome;
use peekbuddy_churn::plan::Shape;
use peekbuddy_churn::profile::{self, Profile, Rate, Verdict};

const USAGE: &str = "usage: cargo xtask churn [--profile <name>] [--seed <n>] [--threads <n>] \
[--rate <ev/s>] [--duration <s>] [--burst <events> --within <s>] [--shape <depth,width,files>] \
[--mix <profile>] [--root <dir>] [--ledger <path>] [--keep]";

/// Options of `cargo xtask churn`.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The workload: the named profile with any overrides applied.
    pub profile: Profile,
    /// Seed of every thread's stream.
    pub seed: u64,
    /// Generator threads.
    pub threads: u32,
    /// Initial tree per thread.
    pub shape: Shape,
    /// Churn root; default: a fresh directory in the temp dir.
    pub root: Option<PathBuf>,
    /// Ledger path; default: `target/churn/<profile>-seed<seed>.jsonl`.
    pub ledger: Option<PathBuf>,
    /// Keep a root the command created.
    pub keep: bool,
}

fn number<T: std::str::FromStr>(option: &str, text: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    text.parse().map_err(|e| format!("{option}: {e}"))
}

fn named(name: &str) -> Result<&'static Profile, String> {
    profile::profile(name).ok_or_else(|| {
        let names: Vec<&str> = profile::PROFILES.iter().map(|p| p.name).collect();
        format!("unknown profile {name}; known: {}", names.join(", "))
    })
}

fn shape(text: &str) -> Result<Shape, String> {
    let parts: Vec<u32> = text
        .split(',')
        .map(|part| number("--shape", part.trim()))
        .collect::<Result<_, _>>()?;
    match parts.as_slice() {
        [depth, width, files] => Ok(Shape {
            depth: *depth,
            width: *width,
            files: *files,
        }),
        _ => Err("--shape takes depth,width,files".to_owned()),
    }
}

/// Parses the options; `--rate`/`--duration` make the profile steady,
/// `--burst`/`--within` make it a burst, `--mix` takes the operation mix
/// of another profile.
///
/// # Errors
///
/// Returns a usage message for unknown options or bad values.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut base = named("edge-mix")?;
    let mut mix = None;
    let (mut rate, mut duration, mut burst, mut within) = (None, None, None, None);
    let mut options = Options {
        profile: base.clone(),
        seed: 1,
        threads: 1,
        shape: Shape::default(),
        root: None,
        ledger: None,
        keep: false,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--profile" => base = named(&value()?)?,
            "--mix" => mix = Some(named(&value()?)?.mix),
            "--seed" => options.seed = number(arg, &value()?)?,
            "--threads" => options.threads = number(arg, &value()?)?,
            "--rate" => rate = Some(number::<u64>(arg, &value()?)?),
            "--duration" => duration = Some(number::<u64>(arg, &value()?)?),
            "--burst" => burst = Some(number::<u64>(arg, &value()?)?),
            "--within" => within = Some(number::<u64>(arg, &value()?)?),
            "--shape" => options.shape = shape(&value()?)?,
            "--root" => options.root = Some(PathBuf::from(value()?)),
            "--ledger" => options.ledger = Some(PathBuf::from(value()?)),
            "--keep" => options.keep = true,
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    if options.threads == 0 {
        return Err("--threads must be at least 1".to_owned());
    }
    options.profile = base.clone();
    if let Some(mix) = mix {
        options.profile.mix = mix;
    }
    options.profile.rate = match (rate.or(duration), burst.or(within)) {
        (Some(_), Some(_)) => {
            return Err("--rate/--duration and --burst/--within exclude each other".to_owned());
        }
        (Some(_), None) => {
            let (base_rate, base_seconds) = match base.rate {
                Rate::Steady {
                    events_per_s,
                    seconds,
                } => (events_per_s, seconds),
                Rate::Burst { events, within_s } => (events / within_s.max(1), within_s),
            };
            Rate::Steady {
                events_per_s: rate.unwrap_or(base_rate),
                seconds: duration.unwrap_or(base_seconds),
            }
        }
        (None, Some(_)) => {
            let (base_events, base_within) = match base.rate {
                Rate::Burst { events, within_s } => (events, within_s),
                Rate::Steady {
                    events_per_s,
                    seconds,
                } => (events_per_s.saturating_mul(seconds), seconds),
            };
            Rate::Burst {
                events: burst.unwrap_or(base_events),
                within_s: within.unwrap_or(base_within),
            }
        }
        (None, None) => base.rate,
    };
    if options.profile.rate.target_per_s() == 0 {
        return Err("the rate must be at least 1 event per second".to_owned());
    }
    Ok(options)
}

#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "the report is rendered by the Windows-only run")
)]
fn seconds(ticks: u64) -> String {
    // FILETIME ticks to seconds for display only.
    #[expect(clippy::cast_precision_loss, reason = "display only")]
    let value = ticks as f64 / 10_000_000.0;
    format!("{value:.2} s")
}

/// The Markdown report of a run.
#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "the report is rendered by the Windows-only run")
)]
#[must_use]
pub fn render(options: &Options, outcome: &Outcome, ledger: &str, timer: &str) -> String {
    let profile = &options.profile;
    let verdict = match outcome.verdict {
        Verdict::Exercised => "exercised",
        Verdict::NotExercised => "**not exercised** (below target: says nothing about budgets)",
    };
    let mut out = format!(
        "## Churn `{}` (seed {}, {} thread(s))\n\n\
         | | |\n| --- | --- |\n\
         | operations | {} |\n\
         | raw events (paced, successful) | {} |\n\
         | paced span | {} |\n\
         | achieved | {} ev/s |\n\
         | target | {} ev/s |\n\
         | verdict | {verdict} |\n\
         | failed operations | {} |\n\
         | sleep timer | {timer} |\n\
         | ledger | `{ledger}` |\n\n\
         | class | ok | failed |\n| --- | --- | --- |\n",
        profile.name,
        options.seed,
        options.threads,
        outcome.records.len(),
        outcome.events,
        seconds(outcome.elapsed),
        outcome.achieved_per_s,
        profile.rate.target_per_s(),
        outcome.failures(),
    );
    for (kind, count) in &outcome.classes {
        let _ = writeln!(out, "| {kind:?} | {} | {} |", count.ok, count.failed);
    }
    let missing = outcome.missing_classes(profile);
    if !missing.is_empty() {
        let _ = writeln!(out, "\n**Never succeeded:** {missing:?}");
    }
    out
}

/// `cargo xtask churn`.
///
/// # Errors
///
/// Returns the report when a class of the mix never succeeded, or a usage
/// or I/O error.
pub fn run(args: &[String]) -> Result<String, String> {
    let options = parse(args)?;
    execute(&options)
}

#[cfg(not(windows))]
fn execute(_: &Options) -> Result<String, String> {
    Err(
        "churn: the generator runs on Windows only (the plan side is tested on every host)"
            .to_owned(),
    )
}

#[cfg(windows)]
fn execute(options: &Options) -> Result<String, String> {
    use peekbuddy_churn::exec::Config;
    use peekbuddy_churn::exec::windows::{WinClock, WinTarget, header};
    use peekbuddy_churn::ledger::to_jsonl;

    let created = options.root.is_none();
    let root = options
        .root
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("pbchurn-{}", std::process::id())));
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let root = std::fs::canonicalize(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if std::fs::read_dir(&root)
        .map_err(|e| format!("{}: {e}", root.display()))?
        .next()
        .is_some()
    {
        return Err(format!("churn: the root {} must be empty", root.display()));
    }
    let runner = ["PEEKBUDDY_RUNNER_LABEL", "ImageOS", "COMPUTERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_else(|| "unknown".to_owned());
    let header = header(
        options.profile.name,
        options.seed,
        options.threads,
        &root,
        &runner,
    );
    // The planner keeps a `&'static Profile`; one leak per command run.
    let profile: &'static Profile = Box::leak(Box::new(options.profile.clone()));
    let config = Config {
        profile,
        seed: options.seed,
        threads: options.threads,
        shape: options.shape,
    };
    let timer = if WinClock::new().high_resolution() {
        "high-resolution waitable timer"
    } else {
        "std::thread::sleep (no high-resolution timer)"
    };
    let outcome = peekbuddy_churn::exec::run(&config, |_| (WinTarget::new(&root), WinClock::new()));
    let ledger = options.ledger.clone().unwrap_or_else(|| {
        PathBuf::from(format!(
            "target/churn/{}-seed{}.jsonl",
            profile.name, options.seed
        ))
    });
    if let Some(dir) = ledger.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = to_jsonl(&header, &outcome.records).map_err(|e| e.to_string())?;
    std::fs::write(&ledger, text).map_err(|e| format!("{}: {e}", ledger.display()))?;
    if created && !options.keep {
        std::fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    let report = render(options, &outcome, &ledger.display().to_string(), timer);
    if let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let mut text = std::fs::read_to_string(&summary).unwrap_or_default();
        text.push_str(&report);
        std::fs::write(&summary, text).map_err(|e| e.to_string())?;
    }
    if outcome.missing_classes(profile).is_empty() {
        Ok(report)
    } else {
        Err(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use peekbuddy_churn::exec::{ClassCount, Outcome};
    use peekbuddy_churn::ops::OpKind;
    use peekbuddy_churn::plan::Shape;
    use peekbuddy_churn::profile::{Rate, Verdict};

    use super::{parse, render};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_are_edge_mix_seed_1_one_thread() {
        let options = parse(&[]).unwrap();
        assert_eq!(options.profile.name, "edge-mix");
        assert_eq!((options.seed, options.threads), (1, 1));
        assert_eq!(options.shape, Shape::default());
        assert!(options.root.is_none() && options.ledger.is_none() && !options.keep);
    }

    #[test]
    fn options_are_read() {
        let options = parse(&args(&[
            "--profile",
            "latency-10k",
            "--seed",
            "7",
            "--threads",
            "4",
            "--shape",
            "1,2,3",
            "--root",
            "r",
            "--ledger",
            "l.jsonl",
            "--keep",
        ]))
        .unwrap();
        assert_eq!(options.profile.name, "latency-10k");
        assert_eq!((options.seed, options.threads), (7, 4));
        assert_eq!(
            options.shape,
            Shape {
                depth: 1,
                width: 2,
                files: 3
            }
        );
        assert_eq!(options.root.unwrap().to_str(), Some("r"));
        assert_eq!(options.ledger.unwrap().to_str(), Some("l.jsonl"));
        assert!(options.keep);
    }

    #[test]
    fn rate_and_burst_overrides() {
        let steady = parse(&args(&["--profile", "latency-10k", "--duration", "5"])).unwrap();
        assert_eq!(
            steady.profile.rate,
            Rate::Steady {
                events_per_s: 10_000,
                seconds: 5
            }
        );
        let rate = parse(&args(&["--rate", "50"])).unwrap();
        assert_eq!(
            rate.profile.rate,
            Rate::Steady {
                events_per_s: 50,
                seconds: 10
            }
        );
        let burst = parse(&args(&["--profile", "burst-100k", "--burst", "5000"])).unwrap();
        assert_eq!(
            burst.profile.rate,
            Rate::Burst {
                events: 5_000,
                within_s: 10
            }
        );
        let from_steady = parse(&args(&["--within", "2"])).unwrap();
        assert_eq!(
            from_steady.profile.rate,
            Rate::Burst {
                events: 2_000,
                within_s: 2
            }
        );
        let to_steady = parse(&args(&["--profile", "burst-100k", "--rate", "10"])).unwrap();
        assert_eq!(
            to_steady.profile.rate,
            Rate::Steady {
                events_per_s: 10,
                seconds: 10
            }
        );
    }

    #[test]
    fn mix_comes_from_another_profile() {
        let options = parse(&args(&["--profile", "latency-10k", "--mix", "edge-mix"])).unwrap();
        assert_eq!(options.profile.mix.len(), OpKind::ALL.len());
        assert_eq!(options.profile.name, "latency-10k");
    }

    #[test]
    fn bad_options_are_rejected() {
        for bad in [
            &["--profile", "nope"][..],
            &["--seed"],
            &["--seed", "x"],
            &["--threads", "0"],
            &["--shape", "1,2"],
            &["--shape", "1,b,3"],
            &["--rate", "5", "--burst", "9"],
            &["--rate", "0"],
            &["--frobnicate"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_report_lists_classes_and_missing_ones() {
        let options = parse(&args(&["--profile", "latency-10k"])).unwrap();
        let mut classes = BTreeMap::new();
        classes.insert(OpKind::CreateFile, ClassCount { ok: 3, failed: 1 });
        let outcome = Outcome {
            records: Vec::new(),
            events: 3,
            elapsed: 15_000_000,
            achieved_per_s: 2,
            verdict: Verdict::NotExercised,
            classes,
        };
        let report = render(&options, &outcome, "l.jsonl", "timer");
        assert!(report.contains("| achieved | 2 ev/s |"), "{report}");
        assert!(report.contains("| target | 10000 ev/s |"), "{report}");
        assert!(report.contains("| paced span | 1.50 s |"), "{report}");
        assert!(report.contains("**not exercised**"), "{report}");
        assert!(report.contains("| failed operations | 1 |"), "{report}");
        assert!(report.contains("| CreateFile | 3 | 1 |"), "{report}");
        assert!(report.contains("**Never succeeded:** [Write,"), "{report}");
    }
}
