# 0002. `ReadDirectoryChangesExW` throughput, overflow and lifecycle

- Issue: #32. Status: complete for the hosted-runner matrix (NTFS, ReFS, exFAT, FAT32, SMB loopback); real UNC servers, Dev Drive and Windows 10 22H2 are untested.
- Harness: `spikes/rdcw-bench` (benchmark) and `spikes/rdcw-bench/run.ps1` (orchestrator), workflow `rdcw-bench`.
- Raw data: `docs/spikes/data/0002/` (one JSON object per cell): `windows-2025.jsonl` and `windows-2025-repeat.jsonl` (two full x64 runs), `windows-11-vs2026-arm.jsonl` (ARM64, throughput and the first burst cells; the job hit its 60-minute limit during the burst sweep), and the `-rest` files (the burst cells for 1 and 4 MiB, the pool cost and the detach case on both runners).

## Environment

| Runner | OS | Build | CPUs |
| --- | --- | --- | --- |
| `windows-2025` | Windows Server 2025 Datacenter | 10.0.26100 | 4 |
| `windows-11-vs2026-arm` | Windows 11 Enterprise ARM64 | 10.0.26200 | 4 |

Every table below is the same on both runners unless a column says otherwise; the matrix, lifecycle and detach results are identical in all full runs (x64 twice, ARM64 once, plus the `-rest` runs). Volumes: the system volume `C:` and scratch VHDs formatted NTFS, ReFS, exFAT and FAT32 (diskpart), plus an SMB loopback share (`\\localhost\share` over a local directory). One watch is one directory handle opened for overlapped listing and associated with an I/O completion port; the buffer is a plain heap allocation, and `FILE_NOTIFY_CHANGE_*` is the full filter (`0x1FF`) with the subtree flag set.

## Verdict

Defaults for the capture engine, each backed by a table below:

| Decision | Value | Evidence |
| --- | --- | --- |
| Buffer, local NTFS | **1 MiB per watch** | Zero overflows at every measured rate for 64 KiB to 4 MiB (one 64 KiB cell in 120 overflowed); 1 MiB tolerated a 100 ms reader gap in every burst cell on both runners, 64 KiB overflowed after 50 ms. 4 MiB costs 2.7 to 4.8 times the CPU per record of 64 KiB for no measured gain. |
| Buffer, UNC and SMB | **64 KiB, plain information class** | 1 MiB is rejected with `ERROR_INVALID_PARAMETER`; 64 KiB works. |
| Information class | **extended (class 2) or full (class 3) on NTFS, plain (class 1) everywhere else** | Classes 2 and 3 fail with `ERROR_INVALID_FUNCTION` (1) on ReFS, exFAT, FAT32 and SMB. |
| Reads in flight per watch | **one**; re-arm straight after the completion | A second buffer (double buffering) gave the same throughput and the same zero overflows; re-arm p50 is 3 to 9 microseconds on x64 and 6 to 8 on ARM64 for buffers up to 1 MiB. |
| Overflow signal | **a successful completion with 0 bytes** | Seen in every overflowing burst cell; recovery is the reconciliation scan of that root. |
| Root loss | **re-arm error 5 after the last completion** | Delete and delete-and-recreate of the root; rename of the root or a parent keeps the watch alive. |
| Volume loss | **no completion arrives**; the engine needs its own liveness signal | A VHD detach under an armed watch left the read pending for the 60 s the probe waited, on x64 (three runs) and ARM64 (one run). |

## Which classes and buffer sizes work (questions 3 and 4)

First completion after one file creation, per volume, information class and buffer size. `ok` means the call armed and delivered one parsed record; the number is the bytes of that completion.

| Volume | Class 1 (plain) | Class 2 (extended) | Class 3 (full) |
| --- | --- | --- | --- |
| NTFS (system and VHD) | ok, 30 bytes | ok, 102 bytes | ok, 102 bytes |
| ReFS (VHD) | ok, 30 bytes | error 1 | error 1 |
| exFAT (VHD) | ok, 30 bytes | error 1 | error 1 |
| FAT32 (VHD) | ok, 30 bytes | error 1 | error 1 |
| SMB loopback, 64 KiB | ok, 30 bytes | error 1 | error 1 |
| SMB loopback, 1 MiB | `InvalidParameter` (no data) | error 1 | error 1 |

- Buffer sizes 64 KiB and 1 MiB behaved identically on every local volume. Over SMB only the 64 KiB buffer worked; 1 MiB is refused even for class 1. The documented network limit is 64 KiB, which is consistent with this; the exact boundary between 64 KiB and 1 MiB was not searched.
- The plain class has no file identity, so on ReFS, exFAT, FAT32 and network paths the event gives only a name and an action. Identity there comes from the reconciliation scan and `OpenFileById` at read time.
- The same record parses for classes 2 and 3 with the parser in `peekbuddy-core` (102 bytes for a short name: an 84-byte header and an 18-byte name). No parse error occurred in any cell.
- The flood columns of the matrix cells are not used: the harness re-armed right before the flood, so the next completion held only the first records. Overflow is measured by the burst experiment below.

## Throughput and re-arm cost (question 1)

Four writer threads create, rename and delete small files in 64 directories; each op produces about 3.7 records. One reader thread dequeues from the completion port, parses every record and re-arms. NTFS VHD, 5 s per cell, 40 cells per runner (4 buffer sizes, single and double buffering, target rates 5k, 20k, 50k, 100k and 200k ops/s). The tables are the mean over the four target rates (20k to 200k ops/s) where the writers saturate; single means one read in flight, double means two buffers re-armed alternately.

windows-2025 (x86-64), first full run:

| Buffer | Mode | Records/s | Records per completion | Re-arm p50 / p99 (us) | Reader cycles per record | Overflows |
| --- | --- | --- | --- | --- | --- | --- |
| 64 KiB | single | 31.7k | 2 | 2.8 / 76 | 4750 | 1 in 4 cells |
| 64 KiB | double | 36.0k | 2 | 2 / 67 | 5070 | 0 |
| 256 KiB | single | 27.4k | 2 | 3 / 91 | 4490 | 0 |
| 256 KiB | double | 35.7k | 2 | 3 / 78 | 5580 | 0 |
| 1 MiB | single | 35.2k | 2 | 4 / 66 | 6660 | 0 |
| 1 MiB | double | 35.2k | 2 | 4 / 70 | 7150 | 0 |
| 4 MiB | single | 35.0k | 2 | 9 / 68 | 13450 | 0 |
| 4 MiB | double | 32.4k | 2 | 10 / 74 | 13620 | 0 |

windows-11-vs2026-arm (ARM64):

| Buffer | Mode | Records/s | Records per completion | Re-arm p50 / p99 (us) | Reader cycles per record | Overflows |
| --- | --- | --- | --- | --- | --- | --- |
| 64 KiB | single | 6.2k | 2 | 6 / 226 | 11600 | 0 |
| 64 KiB | double | 6.1k | 2 | 6 / 243 | 11500 | 0 |
| 256 KiB | single | 6.2k | 2 | 7 / 262 | 12400 | 0 |
| 256 KiB | double | 6.2k | 2 | 6 / 280 | 12300 | 0 |
| 1 MiB | single | 6.0k | 2 | 8 / 230 | 13900 | 0 |
| 1 MiB | double | 6.2k | 2 | 8 / 230 | 15000 | 0 |
| 4 MiB | single | 6.0k | 2 | 16 / 530 | 42400 | 0 |
| 4 MiB | double | 5.8k | 3 | 29 / 1213 | 54700 | 0 |

- **The writers, not the reader, were the limit**: the x64 runner tops out near 9.5k file ops per second (about 36k records per second) however high the target is, and the reader kept up with zero overflows in 119 of 120 throughput cells across the three full runs (the one overflow is the 64 KiB single-buffer cell at 200k target ops/s in the first x64 run). The ARM64 runner's writers top out at about 2.1k ops per second (6.5k records per second at most). The 100k events/s of `ingest.burst` cannot be produced by this harness on this runner, so the notification side of that budget is not confirmed or refuted; it needs a faster writer or a bigger runner (proposed follow-up). The x64 repeat run reproduced the table within about 30 percent (24.9k to 32.7k records/s, reader cycles per record 7.8k to 30k): the spread across VMs is larger than the difference between single and double buffering.
- The records are small: 107 bytes per record on average on x64 (111.5 on ARM64), 3.7 records per file op on x64 and 2.9 on ARM64.
- Records per completion is 2 even at saturation: the reader re-arms in microseconds and gets a completion for almost every event pair. The parse and the dequeue dominate, not the buffer size.
- Reader CPU per record grows with the buffer size. Against 64 KiB the cost is 1.2 to 1.7 times at 1 MiB and 2.7 to 4.8 times at 4 MiB (range over single and double buffering and the three full runs; 256 KiB is 0.95 to 1.14 times), presumably the cost of probing and locking the buffer pages on every re-arm (not measured separately). In the 5k ops/s rows of the first x64 run a 4 MiB buffer costs 15M cycles per 1000 records against 3.2M for 64 KiB. ARM64 spends 1.5 to 2.4 times the cycles per record of x64 at the same buffer size.
- Double buffering did not reduce overflows or raise throughput; it only adds a second buffer per watch.
- Worst re-arm latencies are scheduler stalls: single samples of 50 to 120 ms appear in several cells and are independent of the buffer size. Only the buffer absorbs them.

## Overflow and reader stalls (question 2)

The reader stalls once for 0, 10, 50 or 100 ms after the first completion while the writers keep going at full speed (about 3.9 MB/s of records, estimated from the throughput rows), then reads on. A cell overflows when the completion that follows returns `bytes = 0` with success status. Each cell was run for 10k, 50k and 200k ops (the 1 and 4 MiB cells of the `-rest` runs for 50k only); all agreed in every cell, and the x64 grid was identical in both full runs.

| Buffer | Runner | 0 ms | 10 ms | 50 ms | 100 ms |
| --- | --- | --- | --- | --- | --- |
| 64 KiB | x64 | no | no | **overflow** | **overflow** |
| 64 KiB | ARM64 | no | no | **overflow** | **overflow** |
| 256 KiB | x64 | no | no | no | **overflow** |
| 256 KiB | ARM64 | no | no | no | no |
| 1 MiB | x64 and ARM64 | no | no | no | no |
| 4 MiB | x64 and ARM64 | no | no | no | no |

The first, cancelled ARM64 attempt showed one overflow in three cells at 64 KiB and 10 ms; its data was not kept, so treat 10 ms as borderline for 64 KiB.

The rule that fits the x64 cells: **a read overflows when more bytes of records accumulate during the gap than the buffer holds**, at about 107 bytes per record. At 36k records per second that is 16 ms for 64 KiB, 68 ms for 256 KiB, 270 ms for 1 MiB and 1.1 s for 4 MiB. The ARM64 burst rate was not recorded, so the same arithmetic cannot be checked there; the order of the buffers is the same and ARM64 never overflowed more than x64 did. This is consistent with the kernel keeping the records between two reads and returning them in the next read up to that read's buffer size, and returning the zero-byte completion beyond it. So the engine must treat every zero-byte completion as a gap and reconcile that root, and size the buffer for the longest reader stall it wants to survive at the event rate it expects.

## Directory lifecycle (question 5)

NTFS only (the harness used the extended class, which the other volumes refuse), watch armed on a directory then the directory changed:

| Event | Completion | Re-arm |
| --- | --- | --- |
| Watched root deleted | one more completion (98 bytes, one record) | `ERROR_ACCESS_DENIED` (5) |
| Watched root renamed | one more completion (98 bytes, one record) | ok: the watch follows the directory |
| Parent of the watched root renamed | one more completion (98 bytes, one record) | ok: the watch follows the directory |
| Root deleted and recreated | one more completion (98 bytes, one record) | `ERROR_ACCESS_DENIED` (5): the old handle is dead, open the new directory |

The handle follows the directory object, so a rename of the root or of a parent leaves the watch armed and the record names stay relative to the root; only the absolute path the engine remembers goes stale, so a rename record must trigger a re-resolve of the root path (`GetFinalPathNameByHandle`). Deletion ends the watch: the engine reopens by path with backoff. ReFS lifecycle was not measured because the harness used the extended class there.

## Volume detach (question 6)

A watch armed on a VHD volume, then `diskpart detach vdisk` on the backing file:

- No completion arrived in the 60 s the probe waited (`status = timeout`): the pending read neither fails nor completes when the volume disappears. The same happened in all four runs that reached it (x64 three times, ARM64 once).
- `CancelIoEx` on the handle then completed it with `OperationAborted` and 0 bytes within 10 s.
- Re-arming afterwards failed with `ERROR_FILE_NOT_FOUND` (2).

So a watch cannot detect volume loss by itself. The engine needs an independent liveness signal per watched volume: a device or volume arrival and removal notification, plus a cheap periodic probe (open the root) as the fallback, and it must cancel the read when the signal fires.

## Non-paged pool per armed watch (question 7)

`GetPerformanceInfo().KernelNonpaged` before and after arming `n` watches in separate directories, 500 ms settle. The counter moves by a few pages for unrelated reasons, so the values are an order of magnitude, not exact:

| Runner and run | Buffer | n = 1 | n = 10 | n = 100 |
| --- | --- | --- | --- | --- |
| x64, first run | 64 KiB | 16 KiB per handle | 7.8 KiB | 1.6 KiB |
| x64, first run | 1 MiB | 12 KiB per handle | 7.4 KiB | 3.0 KiB |
| x64, rest run | 64 KiB | 0 | 10.4 KiB | 1.3 KiB |
| x64, rest run | 1 MiB | 0 | 0 | 3.0 KiB |
| ARM64, rest run | 64 KiB | 0 | 0 | 1.0 KiB |
| ARM64, rest run | 1 MiB | 0 | 0 | 2.7 KiB |

An armed watch costs at most a few KiB of non-paged pool (the zeros are runs where the counter did not move or fell) and the amount does not grow with the buffer size, so 100 watches are about 0.1 to 0.3 MiB of non-paged pool in total. The buffer itself is process memory, so the engine's own budget `engine.notify_buffers.total` is what counts.

## Not measured

- Real UNC servers (the SMB loopback share goes through the redirector but not over a network), Dev Drive, Windows 10 22H2.
- Event rates above 36k records per second on x64 and above 6.5k on ARM64 (both writer-bound).
- ARM64 burst cells for 256 KiB at 100 ms are from one run only, and the 1 and 4 MiB ARM64 cells ran 50k ops, not the 10k, 50k and 200k of x64.
- ReFS lifecycle with the plain class.
- Buffer sizes between 64 KiB and 1 MiB over SMB.

## Decision

Applied to the ADR (0002 amendment), `perf/budgets.toml` and #45, #36, #48:

- Local NTFS watches: class 2 or 3, 1 MiB, one read in flight, re-arm immediately.
- Everything else: class 1, and 64 KiB over SMB and UNC.
- A zero-byte completion is an overflow gap and always triggers reconciliation of that root.
- Root deletion reopens the watch with backoff; root or parent rename re-resolves the root path; volume loss is detected outside the read.
- Memory: 1 MiB times the number of local NTFS roots (10 MiB for the 10-root workload) stays well inside `engine.notify_buffers.total` (64 MiB) for the 10-root workload.
