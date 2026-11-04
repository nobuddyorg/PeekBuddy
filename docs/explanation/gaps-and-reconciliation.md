# Gaps and reconciliation

This page explains what the watch list of the viewer means by *Overflowed*
and *Reconciling*, what a **gap** row in the log is, and why PeekBuddy can
say that it never loses a change *silently*. The mechanics are in
[Capture with ReadDirectoryChangesExW](capture-rdcw.md),
[Reconciliation](reconcile.md) and
[What happens when a watch is lost](watch-lifecycle.md); the proof is in
[Zero undetected loss](zero-undetected-loss.md).

## Overflow

Windows tells PeekBuddy about changes by filling a buffer (1 MiB for a local
folder, 64 KiB for a network folder). When changes arrive faster than they
are read, or one burst is larger than the buffer, Windows throws the whole
buffer away and reports only that something was lost. That is an
**overflow**. The watch shows **Overflowed**.

Nothing can bring those changes back from the notification. So PeekBuddy
does two things at once:

1. it writes a **gap row** into the log at the moment the overflow happened,
   so the log itself says "changes may be missing between here and the end of
   the next scan";
2. it starts a **reconcile**: a scan of the watched folder that compares what
   is on disk with what PeekBuddy knew before the overflow.

## Gap

A gap row is a normal row with the kind *gap* or *overflow*. It is drawn with
its own marker, its own row style and an accessible text, so it cannot be
mistaken for a file change. A gap has a start (the row itself) and an end
(the row that closes the reconcile that followed). Everything that happened
inside that window is either in the log already, or will be found by the
scan.

## Reconcile

While the scan runs, the watch shows **Reconciling**. The scan walks the
folder and writes a row for every difference it finds: a file that exists and
was not known (created), one that was known and is gone (deleted), one whose
size or time changed (modified). These rows have the source **RECONCILE** and
carry the time the scan detected the difference, not the time the change
happened, because that time is not known any more. The column tooltip says so.

When the scan finishes, a closing row ends the gap and the watch is **Live**
again. In the viewer, selecting a gap row offers **Show reconcile results**,
which narrows the log to the reconcile rows between that gap and the end of
its scan.

The same scan runs when a lost folder comes back (**Restored**), when a
drive returns after being locked or removed, and after the computer resumes
from sleep, because notifications may have been dropped in all of those.

## What "zero undetected loss" means

PeekBuddy does not claim that Windows never drops a notification. It claims
that **every change is either a row, or inside a gap the log shows and a
reconcile scan has covered**. The number of changes that are neither is
measured on real NTFS volumes with a generator of known churn, and the test
fails if it is not zero. Pausing a watch is not a loss either: the log says
when it was paused and how many changes were counted but not recorded.

What a reconcile cannot recover is the *sequence* inside a gap. If a file was
created and deleted again inside the gap, the scan finds neither. That is why
the gap row exists: it says where the log is incomplete.

## What you can do

- Leave it alone: gaps close themselves.
- If a watch stays **Lost**, check that the folder exists and the drive is
  connected; PeekBuddy keeps trying and records when it is back.
- If the same folder overflows often, watch a smaller part of it, or exclude
  the noisy subfolders (a build output, a cache) in the watch's filters.
