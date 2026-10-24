# 0006. File-system edge cases: RDCW and USN signatures

- Issue: #36. Status: hosted-runner cells done (x64 and ARM64); the Cloud
  Files cells C1 to C8 and the app-exec cell L4 need hardware or accounts the
  runners do not have (see "Not reproducible").
- Harness: `spikes/fs-lab` (`run.ps1` builds the volumes), workflow `fs-lab`.
- Raw data: `docs/spikes/data/0006/{windows-2025,windows-11-vs2026-arm}/`
  (one JSON line per record, names as u16 arrays); fixtures for the
  classifier tests: `crates/peekbuddy-core/tests/fixtures/fs-lab/`.
- Runs: 40141250031 (first data, `windows-2025` Server 2025 build 26100.33650
  and `windows-11-vs2026-arm` Windows 11 Enterprise build 26200.9711).
- Method: the `ReadDirectoryChangesExW` Full class (decoded by the product's
  `notify` parser) on a subtree, plus the privileged USN reader on the same
  volume, three repetitions per scenario. USN is only available on `C:` in
  this run: the VHD volumes had no change journal (fixed in the harness for
  the rerun). The repetitions agree on the VHD volumes; on `C:` they differ
  only by other processes' USN records. x64 and ARM64 produce the same
  RDCW record sequences.

## Findings that change the design

1. **`ReadDirectoryChangesExW` fails on every non-NTFS volume**:
   `ERROR_INVALID_FUNCTION` (1) on ReFS, Dev Drive (ReFS), exFAT and FAT32,
   on both architectures. The extended and full information classes are
   NTFS-only. Capture on those volumes has to fall back to
   `ReadDirectoryChangesW` and the plain class (names and actions only: no
   file id, no sizes, no `FileNameFlags`). The `Class::Plain` parser exists;
   the fs-lab harness now retries with the plain call and the rerun shows what
   it returns (see "Rerun"). Owner of the change: #45/#46.
2. **A case-only rename is three records**: `Removed old`, `RenamedOldName old`,
   `RenamedNewName NEW` (all with `FileNameFlags` 0x80), same file id. A
   consumer that treats `Removed` as a delete reports a deletion that did not
   happen unless it looks ahead for the rename pair (#41, #39). USN reports a
   clean `RENAME_OLD_NAME` / `RENAME_NEW_NAME` / `CLOSE` triple with one FRN.
3. **A write through a hard link reports the path that was used**
   (`link.txt`, name flags 0), while USN reports the first name of the file
   (`orig.txt`) for the same write. A hard link creation is an `Added` for
   the new name with name flags 0x80 and the final size (USN:
   `HARD_LINK_CHANGE` 0x10000 on the new name). Reconciliation by path (#46)
   must treat both names as one file id.
4. **Reparse points are announced twice**: junction and symlink creation is
   an `Added` with attributes 0x10/0x20 and tag 0, followed by a `Modified`
   with the reparse attribute (0x400) and the tag (`0xA0000003` junction,
   `0xA000000C` symlink); USN adds `REPARSE_POINT_CHANGE` 0x100000. A
   classifier that decides on the first record sees a plain file. Removal
   carries the tag. Writes under a junction target outside the watched root
   are not delivered through the junction; writes through a file symlink are
   reported under the *target* path (`target.txt`), and a file created
   through a directory symlink under the real directory (`targetdir\inside.txt`).
   Nothing follows the link: the link itself is never reported as changed
   when its target is written (#46 reparse policy).
5. **Alternate data streams are invisible in RDCW**: creating or appending a
   stream is a `Modified` for the *file* with an unchanged size and no stream
   name, and deleting a stream produces no RDCW record at all. USN reports
   `STREAM_CHANGE` 0x200000 (+ `NAMED_DATA_EXTEND` 0x20 for the append).
   RDCW alone cannot see stream deletes; the reconcile scan does.
6. **Lone UTF-16 surrogates are delivered exactly** in the RDCW name
   (u16 `0xD800`, `0xDC00` preserved on NTFS), in the plain create, write,
   rename and delete records. `String::from_utf16` fails on them; the fixtures
   `N4-*` are the regression input for #39. The harness stored the USN name
   lossily (U+FFFD), so USN-side fidelity is not shown here (harness gap,
   follow-up).
7. **8.3 short names**: a write through the short name reports the short name
   with `FileNameFlags` 2 (DOS) and a rename through it reports the old name
   as `ALONGF~1.TXT` with flag 0x80; the create is reported only under the
   long name (flag 1). There is no second record for the alias on create.
   Whether USN adds records for the alias is **not answered yet**: the VHD
   volume had no journal in this run (rerun).
8. **Per-directory case sensitivity** works as expected: two names differing
   by case in one directory are two files with two file ids (`N3`). One
   `Modified` record for the directory itself appeared in one architecture's
   run and not the other's, so do not rely on it.
9. **Idle noise (B1)**: on the system volume 10 minutes of idle produced
   347 USN records (x64) and 73 (ARM64) from the runner agents
   (`WindowsAzure*.log`, `lastalive*.dat`, PowerShell policy tests). The
   idle NTFS VHD got none in 10 minutes and a Defender custom scan of the
   watched directory produced no record. The 60-minute idle of the issue was
   not run (10 minutes), and Windows Search is not active on the runners.

## Per-scenario signatures

RDCW rows are `Action name (name flags)`; USN rows are reason masks.

| ID | Scenario | RDCW Full class | USN (`C:`) | Proposed class | Confidence |
| --- | --- | --- | --- | --- | --- |
| N1 | write and rename through the 8.3 name | `Modified ALONGF~1.TXT (2)` x2, `RenamedOldName ALONGF~1.TXT (0x80)`, `RenamedNewName` | not captured | user write / rename; resolve alias via `GetLongPathName` | medium |
| N2 | `a.txt` to `A.txt`, also a directory | `Removed a.txt`, `RenamedOldName a.txt`, `RenamedNewName A.txt` (0x80) | `0x1000`, `0x2000`, `0x80002000` | rename (fold `Removed` into the pair) | high |
| N3 | case-sensitive directory | two `Added` with different file ids | not captured | two files | high |
| N4 | lone surrogates | exact u16 in `Added`, `Modified`, rename, `Removed` | name lossy in the capture | user create/write/rename/delete; keep u16 | high |
| L1 | hard link | `Added link.txt (0x80)`; write via link reports `link.txt`; delete link `Removed link.txt (0)` | `0x80010000` on `link.txt`; writes reported as `orig.txt` | link create = create; keep file id | high |
| L2 | junction | `Added`, `Modified` with tag `0xA0000003`; retarget = `Removed` + `Added` + `Modified`; no records for target changes | not captured | reparse point; do not follow | high |
| L3 | symlinks | file link: `Added`, `Modified` tag `0xA000000C` (attr 0x420); dir link attr 0x410; writes at the target path | `0x100`, `0x100100` | reparse point; do not follow | high |
| S1 | alternate data stream | `Modified file.txt` (size unchanged); delete of the stream: nothing | `0x200000`, `0x200020`, `0x20`, `0x200000` | stream change on the file; not a data write | high |
| B1 | idle | none | 347 (x64), 73 (ARM64) records of runner agents | n/a | medium |
| D1 | block cloning | `FSCTL_DUPLICATE_EXTENTS_TO_FILE` succeeded on ReFS and Dev Drive; records only in the rerun | n/a | n/a | pending |

## Exit criteria

- **Every scenario has a fixture or a documented reason**: fixtures for N1 to
  N4, L1 to L3 and S1 (NTFS VHD, x64) plus `C:` USN cuts for N2, N4, L1, L3, S1
  and ARM64 N2, N4, L3. Not reproducible: C1 to C8, L4 (below); D1 and the
  non-NTFS cells pending the rerun.
- **Test over all fixtures**: `fs_lab_fixtures` pins the signatures above on
  Linux. The assertion that classifier *rules* never hide create, delete,
  rename, security or hard-link records, and the 0-allocation replay, need the
  classifier (#49) and `classify.toml`, which do not exist yet; they are
  listed as work for #49, with the fixtures as the corpus.
- **OneDrive `SOURCE_CLIENT_REPLICATION_MANAGEMENT` and USN attributes**:
  not answerable on a runner (no OneDrive). Open.
- **8.3 aliases in USN**: open until the rerun (journal now created).
- **Fixture tests on ubuntu**: yes (`cargo nextest run -p peekbuddy-core --test fs_lab_fixtures`).

## Not reproducible on hosted runners

- C1 to C8 (OneDrive Files On-Demand, hydration, pinning, Storage Sense,
  hidden placeholders): needs a signed-in OneDrive client; owner run.
- L4 (app-exec links during a Store app update): the scenario produced no
  records, the runners do not update Store apps; owner run or a local VM.
- Windows 10 22H2 cells and the 60-minute idle on a desktop with Windows Search.

## Downstream issues

- #49: reparse tag appears only on the second record; classify by
  (attributes, tag) of the latest record; ADS changes are not data writes.
- #39: case-only rename triple, short names flagged 2, lone surrogates in u16.
- #41: rename detection must fold the leading `Removed` of a case-only rename.
- #46: hard-link path versus FRN name; non-NTFS volumes need the plain class;
  stream deletes are only seen by the scan.
- #45: capture has to open non-NTFS volumes with `ReadDirectoryChangesW`.

## Rerun

The harness now falls back to the plain call and creates change journals on
the NTFS, ReFS and Dev Drive volumes. The results (plain-class records on the
four non-NTFS volume types, the 8.3 USN answer, D1 with records) are added
here when run 40141250031's successor finishes.
