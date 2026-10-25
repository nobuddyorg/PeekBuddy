//! `--perf-probe` output (#24, #161).
//!
//! The probe starts the engine role without a viewer, waits for "engine
//! ready", stays up for the requested seconds with no timers of its own
//! besides one one-shot exit timer, then prints one schema-versioned JSON
//! document. The harness (#27) reads it; the schema is
//! `perf/probe.schema.json`.

use std::fmt::Write as _;

use peekbuddy_core::perf::{CounterSnapshot, Milestone, MilestoneTable};

/// `schemaVersion` of the document. Bump it with the schema.
pub const SCHEMA_VERSION: u32 = 1;

/// How many batch commits the engine remembers for latency joins.
pub const COMMIT_RING_CAPACITY: usize = 4096;

/// One process measurement. Times are microseconds since process creation,
/// CPU times are 100 ns ticks as the OS reports them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sample {
    /// Microseconds since process creation when the sample was taken.
    pub at_us: u64,
    /// Commit charge (`PrivateUsage`); never mixed with the working set.
    pub private_bytes: u64,
    /// Working set, including shared pages.
    pub working_set: u64,
    /// Peak working set.
    pub peak_working_set: u64,
    /// Soft and hard page faults.
    pub page_faults: u64,
    /// Kernel CPU time in 100 ns ticks.
    pub kernel_ticks: u64,
    /// User CPU time in 100 ns ticks.
    pub user_ticks: u64,
    /// Open handles.
    pub handles: u64,
    /// GDI objects.
    pub gdi_objects: u64,
    /// USER objects.
    pub user_objects: u64,
    /// Context switches over all threads, when the OS reports them.
    pub context_switches: Option<u64>,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Read operations.
    pub read_ops: u64,
    /// Write operations.
    pub write_ops: u64,
}

/// A milestone as recorded in the in-process table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneRow {
    /// Stable event name, as in ETW.
    pub name: &'static str,
    /// First time recorded.
    pub first_us: u64,
    /// Latest time recorded.
    pub last_us: u64,
    /// How often it was recorded.
    pub count: u32,
}

/// One committed batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchCommit {
    /// Microseconds since process creation at the commit.
    pub at_us: u64,
    /// Rows in the batch.
    pub rows: u64,
}

/// The last [`COMMIT_RING_CAPACITY`] batch commits: bounded, so a long probe
/// never grows without limit. Oldest entries are dropped first.
#[derive(Debug, Default)]
pub struct CommitLog {
    entries: std::collections::VecDeque<BatchCommit>,
}

impl CommitLog {
    /// Creates an empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a commit, dropping the oldest when the log is full.
    pub fn push(&mut self, commit: BatchCommit) {
        if self.entries.len() == COMMIT_RING_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(commit);
    }

    /// The entries, oldest first.
    #[must_use]
    pub fn to_vec(&self) -> Vec<BatchCommit> {
        self.entries.iter().copied().collect()
    }
}

/// Everything the probe prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Package version of the build.
    pub version: String,
    /// Architecture the process runs as.
    pub arch: &'static str,
    /// Seconds the probe was asked to stay up.
    pub requested_seconds: u64,
    /// Milestones that were recorded.
    pub milestones: Vec<MilestoneRow>,
    /// Sample at "engine ready".
    pub start: Sample,
    /// Sample at exit.
    pub end: Sample,
    /// Counters at exit.
    pub counters: CounterSnapshot,
    /// Batch commits, oldest first.
    pub batch_commits: Vec<BatchCommit>,
}

/// The recorded milestones of `table`, in declaration order.
#[must_use]
pub fn milestone_rows(table: &MilestoneTable) -> Vec<MilestoneRow> {
    Milestone::ALL
        .into_iter()
        .filter_map(|milestone| {
            Some(MilestoneRow {
                name: milestone.name(),
                first_us: table.first_us(milestone)?,
                last_us: table.last_us(milestone)?,
                count: table.count(milestone),
            })
        })
        .collect()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

fn sample_json(out: &mut String, sample: &Sample) {
    let switches = sample
        .context_switches
        .map_or_else(|| String::from("null"), |count| count.to_string());
    let _ = write!(
        out,
        "{{\"atUs\":{},\"privateBytes\":{},\"workingSet\":{},\"peakWorkingSet\":{},\
         \"pageFaults\":{},\"kernelTicks\":{},\"userTicks\":{},\"handles\":{},\
         \"gdiObjects\":{},\"userObjects\":{},\"contextSwitches\":{switches},\
         \"readBytes\":{},\"writeBytes\":{},\"readOps\":{},\"writeOps\":{}}}",
        sample.at_us,
        sample.private_bytes,
        sample.working_set,
        sample.peak_working_set,
        sample.page_faults,
        sample.kernel_ticks,
        sample.user_ticks,
        sample.handles,
        sample.gdi_objects,
        sample.user_objects,
        sample.read_bytes,
        sample.write_bytes,
        sample.read_ops,
        sample.write_ops,
    );
}

impl Report {
    /// The JSON document, on one line with a trailing newline.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(1024);
        let _ = write!(
            out,
            "{{\"schemaVersion\":{SCHEMA_VERSION},\"role\":\"engine\",\"version\":\"{}\",\
             \"arch\":\"{}\",\"requestedSeconds\":{},\"milestones\":[",
            escape(&self.version),
            self.arch,
            self.requested_seconds
        );
        for (index, row) in self.milestones.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"name\":\"{}\",\"firstUs\":{},\"lastUs\":{},\"count\":{}}}",
                row.name, row.first_us, row.last_us, row.count
            );
        }
        out.push_str("],\"samples\":{\"start\":");
        sample_json(&mut out, &self.start);
        out.push_str(",\"end\":");
        sample_json(&mut out, &self.end);
        let c = &self.counters;
        let _ = write!(
            out,
            "}},\"counters\":{{\"eventsReceived\":{},\"rowsPersisted\":{},\"overflows\":{},\
             \"gaps\":{},\"ringDepthHighWater\":{},\"lastBatchSize\":{}}},\"batchCommits\":[",
            c.events_received,
            c.rows_persisted,
            c.overflows,
            c.gaps,
            c.ring_depth_high_water,
            c.last_batch_size
        );
        for (index, commit) in self.batch_commits.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"atUs\":{},\"rows\":{}}}",
                commit.at_us, commit.rows
            );
        }
        out.push_str("]}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::perf::{CounterSnapshot, Milestone, MilestoneTable};

    use super::{
        BatchCommit, COMMIT_RING_CAPACITY, CommitLog, Report, Sample, escape, milestone_rows,
    };

    fn sample(seed: u64) -> Sample {
        Sample {
            at_us: seed,
            private_bytes: seed + 1,
            working_set: seed + 2,
            peak_working_set: seed + 3,
            page_faults: seed + 4,
            kernel_ticks: seed + 5,
            user_ticks: seed + 6,
            handles: seed + 7,
            gdi_objects: seed + 8,
            user_objects: seed + 9,
            context_switches: Some(seed + 10),
            read_bytes: seed + 11,
            write_bytes: seed + 12,
            read_ops: seed + 13,
            write_ops: seed + 14,
        }
    }

    pub(crate) fn report() -> Report {
        let table = MilestoneTable::new();
        table.record(Milestone::ProcessStart, 10);
        table.record(Milestone::EngineReady, 900);
        table.record(Milestone::WatcherArmed, 400);
        table.record(Milestone::WatcherArmed, 450);
        Report {
            version: String::from("0.0.0"),
            arch: "x86_64",
            requested_seconds: 5,
            milestones: milestone_rows(&table),
            start: sample(1_000),
            end: Sample {
                context_switches: None,
                ..sample(2_000)
            },
            counters: CounterSnapshot {
                events_received: 7,
                rows_persisted: 6,
                overflows: 1,
                gaps: 1,
                ring_depth_high_water: 3,
                last_batch_size: 2,
            },
            batch_commits: vec![
                BatchCommit {
                    at_us: 500,
                    rows: 2,
                },
                BatchCommit {
                    at_us: 700,
                    rows: 4,
                },
            ],
        }
    }

    #[test]
    fn only_recorded_milestones_are_listed_in_declaration_order() {
        let rows = milestone_rows(&report_table());
        let names: Vec<_> = rows.iter().map(|row| row.name).collect();
        assert_eq!(names, ["ProcessStart", "WatcherArmed", "EngineReady"]);
        let armed = rows.iter().find(|row| row.name == "WatcherArmed").unwrap();
        assert_eq!((armed.first_us, armed.last_us, armed.count), (400, 450, 2));
    }

    fn report_table() -> MilestoneTable {
        let table = MilestoneTable::new();
        table.record(Milestone::EngineReady, 900);
        table.record(Milestone::ProcessStart, 10);
        table.record(Milestone::WatcherArmed, 400);
        table.record(Milestone::WatcherArmed, 450);
        table
    }

    #[test]
    fn the_document_is_one_line_of_valid_json() {
        let json = report().to_json();
        assert!(json.ends_with("}\n"));
        assert_eq!(json.matches('\n').count(), 1);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            value["samples"]["end"]["contextSwitches"],
            serde_json::Value::Null
        );
        assert_eq!(value["samples"]["start"]["contextSwitches"], 1_010);
        assert_eq!(value["batchCommits"][1]["rows"], 4);
        assert_eq!(value["counters"]["overflows"], 1);
    }

    #[test]
    fn the_commit_log_is_bounded_and_drops_the_oldest() {
        let mut log = CommitLog::new();
        for index in 0..(COMMIT_RING_CAPACITY as u64 + 10) {
            log.push(BatchCommit {
                at_us: index,
                rows: 1,
            });
        }
        let kept = log.to_vec();
        assert_eq!(kept.len(), COMMIT_RING_CAPACITY);
        assert_eq!(kept[0].at_us, 10);
        assert_eq!(
            kept[COMMIT_RING_CAPACITY - 1].at_us,
            COMMIT_RING_CAPACITY as u64 + 9
        );
    }

    fn schema() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../perf/probe.schema.json")).unwrap()
    }

    #[test]
    fn the_document_validates_against_the_committed_schema() {
        let validator = jsonschema::validator_for(&schema()).unwrap();
        let document: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&document)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn the_schema_refuses_a_document_with_a_missing_or_extra_field() {
        let validator = jsonschema::validator_for(&schema()).unwrap();
        let mut document: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
        document["surprise"] = serde_json::Value::Bool(true);
        assert!(!validator.is_valid(&document));
        document.as_object_mut().unwrap().remove("surprise");
        document.as_object_mut().unwrap().remove("counters");
        assert!(!validator.is_valid(&document));
    }

    /// Keeps keys and shapes, replaces every number with `0`.
    fn skeleton(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Number(_) => serde_json::Value::from(0),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().take(2).map(skeleton).collect())
            }
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(key, item)| (key.clone(), skeleton(item)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    #[test]
    fn the_structure_is_pinned() {
        let document: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
        insta::assert_json_snapshot!(skeleton(&document));
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(escape("a\"b\\c\n"), "a\\\"b\\\\c\\u000a");
    }
}
