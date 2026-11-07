//! Query-layer benchmarks at the scale of the budgets (#267, #26): the viewer
//! scrolls and filters a log of millions of rows.
//!
//! A store of this size takes minutes to build, so these benches do nothing
//! unless `PEEKBUDDY_QUERY_ROWS` says how many rows to make; the A/B and
//! trend runs of the other benches see no benchmark in this file. The
//! nightly `query bench` job runs it twice, with 1,000,000 and with
//! 10,000,000 rows. `PEEKBUDDY_QUERY_DIR` names a directory to keep the
//! store in between runs (it is reused when it holds the same number of rows).
//!
//! | bench | rows | budget row |
//! | --- | --- | --- |
//! | `query/page_keyset_10m` | 10M | `bench.query.page_keyset_10m` (<= 2 ms) |
//! | `query/fts_lookup_10m` | 10M | `bench.query.fts_lookup_10m` |
//! | `query/subtree_resolve_10m` | 10M | `bench.query.subtree_resolve_10m` |
//! | `query/regex_post_filter_1m` | 1M | `bench.query.regex_post_filter_1m` |
//! | `query/filtered_first_1m` | 1M | `bench.query.filtered_first_1m` (<= 100 ms) |

use std::sync::Arc;

use criterion::{Criterion, Throughput};
use peekbuddy_core::filter::{FilterSet, Pattern, Rule};
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
use peekbuddy_store::read::Reader;

#[path = "support/store.rs"]
mod fixture;

use fixture::{store, units};

/// The id a page from the middle of the log starts at.
fn middle(reader: &Reader, cancel: &CancelFlag, rows: u64) -> Option<i64> {
    let newest = reader
        .page(&PageRequest::newest(0, Filter::default()), cancel)
        .ok()?
        .newest_id()?;
    newest.checked_sub(i64::try_from(rows / 2).ok()?)
}

fn page_request(cursor: Option<i64>, filter: Filter) -> PageRequest {
    PageRequest {
        generation: 1,
        filter,
        cursor,
        direction: Direction::Older,
        limit: 200,
    }
}

fn main() {
    let Some(rows) = std::env::var("PEEKBUDDY_QUERY_ROWS")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|rows| *rows > 0)
    else {
        return;
    };
    let Some((dir, _guard)) = store(rows) else {
        return;
    };
    let reader = Reader::new(&dir);
    let cancel = CancelFlag::new();
    let mut c = Criterion::default().configure_from_args();
    let mut group = c.benchmark_group("query");
    let at = middle(&reader, &cancel, rows);

    if rows >= 10_000_000 {
        // A keyset page from the middle of the log: 200 rows, paths built.
        group.throughput(Throughput::Elements(200));
        let request = page_request(at, Filter::default());
        group.bench_function("page_keyset_10m", |b| {
            b.iter(|| reader.page(&request, &cancel));
        });
        // A word that one row in a thousand has: the first page of hits, found
        // through the trigram index and ordered by id without a sort.
        let fts = page_request(
            None,
            Filter {
                name: Some(String::from("rareword")),
                ..Filter::default()
            },
        );
        group.bench_function("fts_lookup_10m", |b| {
            b.iter(|| reader.page(&fts, &cancel));
        });
        // One directory in ten thousand: the subtree is resolved and its rows
        // are found by walking the ids.
        let subtree = page_request(
            None,
            Filter {
                subtree: Some(units(r"C:\Watched\top5\sub5")),
                ..Filter::default()
            },
        );
        group.bench_function("subtree_resolve_10m", |b| {
            b.iter(|| reader.page(&subtree, &cancel));
        });
    }

    if rows <= 2_000_000 {
        let regex = FilterSet::compile(&[Rule::include(Pattern::Regex(String::from(
            r"file\d*50007-rareword\.dat$",
        )))]);
        if let Ok(set) = regex {
            let filter = Filter {
                pattern: Some(Arc::new(set)),
                ..Filter::default()
            };
            // The whole log through a regex, in time boxed steps: rows per
            // second of the post-filter.
            group.throughput(Throughput::Elements(rows));
            group.sample_size(10);
            group.bench_function("regex_post_filter_1m", |b| {
                b.iter(|| {
                    let mut cursor = None;
                    loop {
                        let Ok(page) = reader.page(&page_request(cursor, filter.clone()), &cancel)
                        else {
                            return 0;
                        };
                        match page.resume {
                            Some(next) => cursor = Some(next),
                            None => return page.scanned,
                        }
                    }
                });
            });
            // Time to the first filtered rows with a glob that one row in a
            // thousand matches: calls until one has a row.
            let glob =
                FilterSet::compile(&[Rule::include(Pattern::Glob(String::from("*-rareword.dat")))]);
            let Ok(glob) = glob else {
                return;
            };
            let glob = Filter {
                pattern: Some(Arc::new(glob)),
                ..Filter::default()
            };
            group.throughput(Throughput::Elements(1));
            group.bench_function("filtered_first_1m", |b| {
                b.iter(|| {
                    let mut cursor = None;
                    loop {
                        let Ok(page) = reader.page(&page_request(cursor, glob.clone()), &cancel)
                        else {
                            return 0;
                        };
                        if !page.rows.is_empty() {
                            return page.rows.len();
                        }
                        match page.resume {
                            Some(next) => cursor = Some(next),
                            None => return 0,
                        }
                    }
                });
            });
        }
    }
    group.finish();
    c.final_summary();
}
