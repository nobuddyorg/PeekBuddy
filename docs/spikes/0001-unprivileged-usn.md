# 0001. Unprivileged USN journal reading on current Windows

- Issue: #31. Status: complete for the hosted-runner matrix; Windows 10 22H2 and Dev Drive untested.
- Harness: `spikes/usn-probe` (probe) and `spikes/usn-probe/run.ps1` (orchestrator), workflow `usn-probe`.
- Raw data: `docs/spikes/data/0001/` (one JSON object per attempt).

## Environment

| Runner | OS | Build | CPUs |
| --- | --- | --- | --- |
| `windows-2025` | Windows Server 2025 Datacenter | 10.0.26100 (the 24H2 kernel) | 4 |
| `windows-11-vs2026-arm` | Windows 11 Enterprise ARM64 | 10.0.26200 (25H2) | 4 |

The probe ran in three tokens on every volume:

- `admin`: the runner's full administrator token (UAC is off on the images; `TokenElevation` reads 0, integrity high).
- `restricted`: a restricted-token child (Administrators deny-only, no privileges, medium integrity).
- `user`: a real local standard user (`net user`, member of `Users` only, started through `CreateProcessWithLogonW`, medium integrity).

Volumes: the system volume `C:` (NTFS), a scratch VHD formatted NTFS, a scratch VHD formatted ReFS (each with a journal created by `fsutil usn createjournal`), a VHD formatted exFAT and one formatted FAT32 as negative controls, and the data drive `D:` (NTFS without a journal). Each cell is one run; the x64 and ARM64 runners gave identical results for every pass/fail cell.

## Verdict

**By the decision rule of #31 the answer is NO-GO for USN as the primary v1 source, because names are never returned.** Everything else in the rule passes:

| Rule item | Result |
| --- | --- |
| Works for a real standard user on 24H2 (26100) NTFS | yes, with a `guid-dir`, `traverse` or `backup-root` handle |
| Works for a real standard user on 25H2 (26200) NTFS | yes |
| Returns names for accessible files | **no: every record has an empty name, for every token including the administrator** |
| Can be issued overlapped and cancelled | yes (`CancelIoEx` ends it with error 995) |
| Change to readable record p99 <= 50 ms on NTFS | yes (p99 3.4 to 5.9 ms on `C:`, worst single sample 31 ms) |

The unprivileged FSCTL is real and usable, so the ADR's condition "unelevated reading works" is met; the written rule in the spike additionally requires names. The records carry the file reference number (FRN) and the parent FRN, never the name, so a USN backend needs the FRN-to-path map of the capture design plus `OpenFileById` at read time, which cannot name deleted files or files the caller cannot open (see the resolution table). The recommendation recorded in the ADR: v1 keeps RDCW with the reconciliation scan as the capture path; USN stays an optional backend for the service mode (#79) and for a later FRN-keyed completeness check. The owner can overrule the rule; the numbers below are what that decision would rest on.

## Handles and FSCTLs for a standard user

Same on both runners.

NTFS (`C:` and the NTFS VHD):

| Handle | `FSCTL_QUERY_USN_JOURNAL` | `FSCTL_GET_NTFS_VOLUME_DATA` | `FSCTL_ENUM_USN_DATA` | `FSCTL_READ_USN_JOURNAL` | `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` |
| --- | --- | --- | --- | --- | --- |
| `guid-dir` | ok | ok | error 5 | error 5 | ok |
| `traverse` | ok | ok | error 5 | error 5 | ok |
| `backup-root` | ok | ok | error 5 | error 5 | ok |
| `device-read` | open fails (5) | - | - | - | - |

ReFS (VHD):

| Handle | `FSCTL_QUERY_USN_JOURNAL` | `FSCTL_GET_NTFS_VOLUME_DATA` | `FSCTL_ENUM_USN_DATA` | `FSCTL_READ_USN_JOURNAL` | `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` |
| --- | --- | --- | --- | --- | --- |
| `guid-dir` | ok | error 1 | error 1 | error 5 | ok |
| `traverse` | ok | error 1 | error 1 | ok | ok |
| `backup-root` | ok | error 1 | error 1 | error 5 | ok |
| `device-read` | ok | error 1 | error 1 | ok | ok |

Observations:

- Three handle variants work for the unprivileged read: the volume GUID path with a trailing slash opened as a directory (`guid-dir`), `\\.\C:` opened with `FILE_TRAVERSE` (`traverse`), and `C:\` with backup semantics (`backup-root`). The documented `\\.\C:` with `GENERIC_READ` (`device-read`) fails to open for a standard user on NTFS (error 5) and is the only handle that does not.
- `FSCTL_QUERY_USN_JOURNAL`, `FSCTL_GET_NTFS_VOLUME_DATA` (NTFS only) and `FSCTL_READ_FILE_USN_DATA` work unelevated through the same handles. `FSCTL_ENUM_USN_DATA` and the privileged `FSCTL_READ_USN_JOURNAL` are refused with error 5 on NTFS: a full MFT enumeration is not available, so the initial FRN map must come from a directory walk (`FileIdExtdDirectoryInfo`).
- **On ReFS the privileged `FSCTL_READ_USN_JOURNAL` also works for a standard user** through the `traverse` and `device-read` handles, and those records carry names (10 of 10 and 22 of 22). This is the one path that returned names to a non-administrator. It was seen on a diskpart-formatted ReFS VHD on Server 2025 and Windows 11 Enterprise only; Dev Drive and other client SKUs are untested, so it is a lead and not a basis for a decision.
- exFAT and FAT32 return error 1 (`ERROR_INVALID_FUNCTION`) for every FSCTL: the negative control holds. The NTFS data drive without a journal returns error 1179 (`ERROR_JOURNAL_NOT_ACTIVE`).

## Names and access (questions 3 and 4)

The admin side changed files in a directory only Administrators and SYSTEM can use, and files with a deny-read ACL; the probe read the journal afterwards. Records returned (p. 1: all records since the probe's start USN):

| Cell | windows-2025 | windows-11-vs2026-arm |
| --- | --- | --- |
| NTFS C:, admin | 107 records, 107 empty names, 45 under the denied dir | 108 records, 108 empty names, 45 under the denied dir |
| NTFS C:, restricted | 107 records, 107 empty names, 45 under the denied dir | 151 records, 151 empty names, 45 under the denied dir |
| NTFS C:, user | 109 records, 109 empty names, 45 under the denied dir | 109 records, 109 empty names, 45 under the denied dir |
| ReFS VHD, admin | 107 records, 107 empty names, 45 under the denied dir | 107 records, 107 empty names, 45 under the denied dir |
| ReFS VHD, restricted | 107 records, 107 empty names, 45 under the denied dir | 107 records, 107 empty names, 45 under the denied dir |
| ReFS VHD, user | 107 records, 107 empty names, 45 under the denied dir | 107 records, 107 empty names, 45 under the denied dir |

- Every record has an FRN and a parent FRN (none zero) and an empty name.
- Records for files the caller cannot access are still delivered: 45 records had the denied directory as parent, for the restricted token and the standard user too. Nothing is hidden; only names are absent for everyone.

Resolving record FRNs with `OpenFileById` plus `GetFinalPathNameByHandleW` (the cache-miss path of #47), through the volume handle:

| Cell | windows-2025 | windows-11-vs2026-arm |
| --- | --- | --- |
| NTFS C:, admin | 27 of 107, own 9 of 9, errors {87: 80} | 28 of 108, own 9 of 9, errors {87: 80} |
| NTFS C:, restricted | 6 of 107, own 6 of 6, errors {5: 21, 87: 80} | 48 of 151, own 6 of 6, errors {5: 21, 87: 82} |
| NTFS C:, user | 6 of 109, own 6 of 6, errors {5: 21, 87: 82} | 6 of 109, own 6 of 6, errors {5: 21, 87: 82} |
| ReFS VHD, admin | 27 of 107, own 9 of 9, errors {87: 80} | 27 of 107, own 9 of 9, errors {87: 80} |
| ReFS VHD, restricted | 6 of 107, own 6 of 6, errors {5: 21, 87: 80} | 6 of 107, own 6 of 6, errors {5: 21, 87: 80} |
| ReFS VHD, user | 6 of 107, own 6 of 6, errors {5: 21, 87: 80} | 6 of 107, own 6 of 6, errors {5: 39, 87: 62} |

- It works for files that still exist and that the caller can open (own files: all resolved).
- Error 87 (`ERROR_INVALID_PARAMETER`) is a file that no longer exists: deleted and renamed-over files cannot be named afterwards, so deletions need a cached FRN-to-path map.
- Error 5 is a file the caller cannot open (the denied directory): it can be counted but not named.

## Latency and cancellation (questions 5 and 6)

Change to readable record, 200 file creations per cell, polling with `BytesToWaitFor = 1`, matched by FRN, microseconds as p50 / p99 / max:

| Cell | windows-2025 | windows-11-vs2026-arm |
| --- | --- | --- |
| NTFS C:, admin | 341 / 3748 / 12638 | 269 / 4323 / 5940 |
| NTFS C:, restricted | 359 / 4152 / 8657 | 499 / 5144 / 8917 |
| NTFS C:, user | 299 / 3446 / 4477 | 291 / 5864 / 7035 |
| NTFS VHD, admin | 189 / 451 / 668 | 172 / 4156 / 5564 |
| NTFS VHD, restricted | 195 / 482 / 562 | 186 / 3206 / 6184 |
| NTFS VHD, user | 195 / 512 / 743 | 176 / 3537 / 5993 |
| ReFS VHD, admin | 193 / 269 / 370 | 359 / 487 / 493 |
| ReFS VHD, restricted | 192 / 276 / 278 | 394 / 635 / 1127 |
| ReFS VHD, user | 190 / 278 / 310 | 391 / 1907 / 2322 |

The ~30 s buffering reported for Dev Drive was not seen on the ReFS VHD (p99 0.3 to 1.9 ms); Dev Drive itself is untested.

An overlapped unprivileged read at the journal head pends (`ERROR_IO_PENDING`), does not complete on its own, and `CancelIoEx` ends it with error 995 about 0.5 s later in every cell (the probe waits 0.5 s before cancelling). IOCP association was not exercised; the overlapped handle works with an event.

## Not measured

- Windows 10 22H2 (no hosted image) and a real client SKU for 24H2: Server 2025 stands in for 24H2.
- Dev Drive and a ReFS volume that was not a diskpart VHD.
- Question 7 (the directory FRN set for a 100k-directory subtree: build time and memory).
- Whether the names are returned by a newer build or by an elevated token with the privilege held (the administrator token here did not get names either).

## Decision

Applied to the ADR (0002 amendment), #47 and #79:

- v1 capture: RDCW plus reconciliation. USN does not join v1 as a primary source.
- #47 (USN backend): if it is built, the recipe is a `guid-dir` handle, `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`, FRN and parent FRN only, names from a cached FRN-to-path map seeded by a directory walk and refreshed with `OpenFileById` on cache misses; deleted files are named from the cache only. Startup self-test: query the journal and read once; on failure fall back to RDCW and log a watch-state event.
- #79 (service mode): USN with the privileged read returns names for administrators, so it is an option for the service, not a requirement.
