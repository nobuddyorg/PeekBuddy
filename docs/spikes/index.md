# Spike reports

Measurements that answer a question before building (#30). Each report
states the question, the method, the numbers and the decision.

| Report | Question | Issue |
| --- | --- | --- |
| [0001 Unprivileged USN](0001-unprivileged-usn.md) | Can a standard user tail the USN journal? | #31 |
| [0002 RDCW throughput](0002-rdcw-throughput.md) | Which buffer size, class and re-arm strategy keeps `ReadDirectoryChangesExW` lossless, and what happens at overflow, root loss and volume detach? | #32 |
| [0006 File-system edge cases](0006-fs-edge-cases.md) | Which RDCW and USN records do short names, case renames, links, reparse points and streams produce, and which volumes support the Ex call? | #36 (in progress) |
| [0003 Viewer renderer](0003-viewer-renderer.md) | glow or wgpu/DX12 for the egui viewer, and what does it cost in memory, first frame and frame time? | #33 (hosted cells done, owner run left) |
| 0004 CI runners | What can GitHub-hosted Windows runners do? | #34 (in progress) |
| [0005 SQLite ingest and query](0005-sqlite-ingest-query.md) | What do the commit policy, pragmas, indexes and FTS really cost on Windows? | #35 (done, one owner-PC run in, NVMe and HDD open) |
