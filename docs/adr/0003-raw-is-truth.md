# 0003. Store every raw record; derive everything else as views

- Status: Accepted
- Date: 2026-10-22
- Implemented or verified by: #38, #41, #53, #66

## Context and problem statement

Raw change records are noisy: an editor's atomic save is a create, a few
writes, a rename and a delete; a rename is two records; cloud sync clients
produce churn. Users want the readable story, but coalescing at ingest would
hide records forever and make loss undetectable.

## Decision drivers

- Zero undetected loss (ADR 0002) must survive every transformation.
- Users need the readable view by default.

## Considered options

1. Coalesce and stitch at ingest, store the result.
2. Store raw records only; compute stitching, atomic saves and coalescing as
   views (F3).

## Decision outcome

Chosen option 2. One row per raw record. Rename stitching, atomic-save
detection, per-path coalescing and cloud-churn hiding are derived views. Every
derived row shows how many raw records it stands for, and hidden cloud churn
is tagged and counted, never dropped.

## Consequences

- Good: the raw log can always be audited; views can change without migrating
  data.
- Bad: more rows to store; the store budgets (#52-#54) must assume raw volume.

## Evidence

- Design discussion in the roadmap #1, decision register F3, checked 2026-10-22.
