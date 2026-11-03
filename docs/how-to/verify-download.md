# Verify a download

PeekBuddy v1 is not code-signed ([why](../explanation/smartscreen-and-smart-app-control.md)),
so Windows cannot tell you who built the file. These two checks can: each
release carries a list of checksums and a GitHub attestation for every file.

You do not need to trust a website for either check; they ask GitHub and your
own PC.

## 1. The checksum

Every release has a file named `SHA256SUMS` with one line per asset. In
PowerShell, in the folder of your download:

```powershell
Get-FileHash .\peekbuddy-<version>-x86_64-pc-windows-msvc.zip -Algorithm SHA256
```

Compare the hash with the line of the same file name in `SHA256SUMS`. They must
be identical. If they differ, do not run the file: download it again, and if it
still differs, [report it](../../SECURITY.md).

A matching checksum says the file is the one that was published, byte for
byte. It does not say who published it; `SHA256SUMS` sits next to the file.
The attestation below closes that gap.

## 2. The attestation

An attestation is a signed statement by GitHub that this file was built by a
workflow of this repository, from a named commit. With the
[GitHub CLI](https://cli.github.com/):

```powershell
gh attestation verify .\peekbuddy-<version>-x86_64-pc-windows-msvc.zip --repo nobuddyorg/PeekBuddy
```

A good result lists the repository, the workflow and the commit. A file that
was changed after the build, or that came from somewhere else, fails with a
message and a non-zero exit code.

What it proves: the file was built by `nobuddyorg/PeekBuddy`'s release
workflow. What it does not prove: that the code is free of bugs. The source is
public, and the build is described in the [release runbook](release.md).

## Do both on every release

SmartScreen keeps its reputation per file hash for unsigned files, so every
release starts from zero. That is one more reason to check the hash: a
warning on a new release is expected, a mismatch is not.

## If a check fails

- Hash mismatch: the download is damaged or not the published file. Download
  again from the release page. Do not run it.
- Attestation fails: do not run the file. Report it, with the file name and the
  output of the command.
- Windows Security removes the file: see
  [SmartScreen and Smart App Control](../explanation/smartscreen-and-smart-app-control.md#defender-and-false-positives).
