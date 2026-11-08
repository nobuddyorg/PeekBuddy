use std::collections::HashMap;

use proptest::prelude::*;

use super::*;

fn truth(stream: &[u64]) -> HashMap<u64, u64> {
    let mut counts = HashMap::new();
    for &item in stream {
        *counts.entry(item).or_insert(0) += 1;
    }
    counts
}

fn sketch_of(capacity: usize, stream: &[u64]) -> Sketch {
    let mut sketch = Sketch::new(capacity);
    for &item in stream {
        sketch.offer(item);
    }
    sketch
}

#[test]
fn a_stream_with_fewer_items_than_counters_is_exact() {
    let sketch = sketch_of(4, &[1, 2, 1, 3, 1, 2]);
    assert_eq!(
        sketch.top(),
        [
            Counter {
                key: 1,
                count: 3,
                error: 0
            },
            Counter {
                key: 2,
                count: 2,
                error: 0
            },
            Counter {
                key: 3,
                count: 1,
                error: 0
            },
        ]
    );
    assert_eq!(sketch.seen(), 6);
}

#[test]
fn a_new_item_takes_the_smallest_counter_and_remembers_the_overcount() {
    let sketch = sketch_of(2, &[1, 1, 1, 2, 3]);
    // 3 replaced 2 (count 1): count 2, error 1.
    assert_eq!(
        sketch.top(),
        [
            Counter {
                key: 1,
                count: 3,
                error: 0
            },
            Counter {
                key: 3,
                count: 2,
                error: 1
            },
        ]
    );
}

#[test]
fn ties_come_out_in_key_order() {
    let sketch = sketch_of(3, &[9, 5, 7]);
    let keys: Vec<_> = sketch.top().iter().map(|c| c.key).collect();
    assert_eq!(keys, [5, 7, 9]);
}

#[test]
fn a_capacity_of_zero_is_one() {
    let sketch = sketch_of(0, &[1, 2, 3]);
    assert_eq!(sketch.top().len(), 1);
}

#[test]
fn clearing_forgets_the_items_and_keeps_the_room() {
    let mut sketch = sketch_of(3, &[1, 2, 3, 4]);
    sketch.clear();
    assert_eq!(sketch.seen(), 0);
    assert_eq!(sketch.top(), []);
    sketch.offer(8);
    assert_eq!(
        sketch.top(),
        [Counter {
            key: 8,
            count: 1,
            error: 0
        }]
    );
}

#[test]
fn the_error_bound_is_the_stream_over_the_capacity() {
    let sketch = sketch_of(4, &[1; 10]);
    assert_eq!(sketch.max_error(), 3);
    assert_eq!(Sketch::new(100).max_error(), 0);
}

proptest! {
    /// The counters add up to the stream; no counter is more than N/K too
    /// high; the truth lies between count - error and count; every item
    /// above N/K is followed.
    #[test]
    fn the_space_saving_guarantees_hold(
        capacity in 1_usize..12,
        stream in proptest::collection::vec(0_u64..30, 0..400),
    ) {
        let sketch = sketch_of(capacity, &stream);
        let top = sketch.top();
        let n = stream.len() as u64;
        prop_assert_eq!(top.iter().map(|c| c.count).sum::<u64>(), n);
        prop_assert!(top.len() <= capacity);
        let exact = truth(&stream);
        for counter in &top {
            let real = exact.get(&counter.key).copied().unwrap_or(0);
            prop_assert!(counter.count >= real, "{counter:?} real {real}");
            prop_assert!(counter.count - counter.error <= real, "{counter:?} real {real}");
            prop_assert!(counter.error <= sketch.max_error(), "{counter:?}");
        }
        for (&key, &real) in &exact {
            if real > n / capacity as u64 {
                prop_assert!(top.iter().any(|c| c.key == key), "{key} ({real} of {n}) lost");
            }
        }
    }

    /// The busiest followed item is never below the true busiest, and not
    /// above it by more than the error bound.
    #[test]
    fn the_top_item_is_close_to_the_truth(
        stream in proptest::collection::vec(0_u64..50, 1..500),
    ) {
        let sketch = sketch_of(10, &stream);
        let best = truth(&stream).values().copied().max().unwrap_or(0);
        let top = sketch.top().first().map_or(0, |c| c.count);
        prop_assert!(top >= best);
        prop_assert!(top - best <= sketch.max_error());
    }
}
