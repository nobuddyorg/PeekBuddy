# Zero undetected loss

PeekBuddy does not promise that no notification is ever dropped; Windows can
drop them. It promises that **a loss is never silent**: every change is
either a row in the log, or inside a gap that the log shows, followed by a
reconcile scan that found what the gap hid. The count of changes that are
neither is the number this suite keeps at zero.

## The test

`crates/peekbuddy/tests/fs_serial_zero_loss.rs` starts the real
`peekbuddy.exe` (headless) on a settings file that watches one folder, runs the
churn generator (#25) on that folder, lets the log settle, stops the engine
and reads the segments back through the query layer. The comparator of the
churn crate then checks the ledger against the rows:

- an operation is **matched** when every raw record it must produce is a row
  with the same path and kind, logged at or after the operation started;
- it is **covered** when some are missing but a gap row precedes a finished
  reconcile that ended after the operation did;
- anything else is an **undetected loss**, and the test fails on the first one.

The tests run in the `fs-serial` group, one at a time, on a real NTFS volume.

| Test | Workload | What it proves |
| --- | --- | --- |
| `every_churn_operation_is_logged_or_covered_by_a_gap` | `edge-mix`, 2 threads, every operation class, exotic names | no loss on the normal path |
| `a_forced_overflow_is_a_gap_followed_by_a_reconcile` | `burst-50k`, 4 threads, a 64 KiB buffer, the engine frozen while the churn runs | at least one gap, each one followed by a finished reconcile, no loss outside it |

Freezing the engine (`peekbuddy_win::suspend`) makes the overflow certain on
any machine; a fast disk and a fast CPU would otherwise keep up with the
burst.

## What a cross-directory move looks like

NTFS reports a move to another directory as a removal at the old path and an
addition at the new one, not as a rename pair. The first run of this suite
showed it for every move and the ledger's expectation was corrected: a rename
within one directory is `renamed_old` and `renamed_new`, a move is `removed`
and `added`.

## Not covered yet

The nightly tier (1M operations, root deletion, kill mid-burst), the USN tier
on a VHD, the restricted-token run and the final-tree comparison after a gap
are not in this change. The comparator treats a gap as covering the whole
root; per-subtree gaps need the reconcile rows to name their subtree.
