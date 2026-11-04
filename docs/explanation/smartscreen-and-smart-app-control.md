# SmartScreen and Smart App Control

PeekBuddy v1 is not code-signed. This page says why, what Windows will show,
and what each choice costs you. Nothing here asks you to lower your security
beyond what you decide yourself.

## Why unsigned

PeekBuddy is a hobby project, and it will never pay for a code-signing
certificate ([ADR 0012](../adr/0012-code-signing.md)). Free signing (the
SignPath Foundation for the GitHub builds, and a Microsoft Store package that
the Store signs) is tried after v1. If neither works, PeekBuddy stays unsigned
and this page is the permanent answer.

Instead of a signature, every release has a checksum list and a GitHub
attestation for each file: [verify a download](../how-to/verify-download.md).

## SmartScreen: "Windows protected your PC"

The first time you start `peekbuddy.exe`, SmartScreen may show *Windows
protected your PC*, with *Unknown publisher*. That is the normal reaction to
an unsigned file that few people have run yet. It judges unsigned files by the
hash of the file, so every new release starts again.

To continue after you verified the download: choose **More info**, then
**Run anyway**. You are trusting the file you verified, so verify first.

## Smart App Control

Smart App Control (SAC) is stricter. Where it is **on**, Windows blocks an
unsigned executable outright: there is no *Run anyway*, and PeekBuddy will
not start.

Check whether it is on: **Windows Security > App & browser control > Smart App
Control**. It reads *On*, *Evaluation* or *Off*.

If it is on you have three options:

1. Do not run PeekBuddy on this PC. This is a fine choice.
2. Wait for a signed build (after v1).
3. Turn SAC off. That is a security trade-off for the whole PC, not just for
   PeekBuddy: SAC protects against unknown programs of any kind. Since the
   Windows update KB5083769 you can turn it back on later; before that
   update, switching it off was permanent until Windows was reinstalled. Read
   Microsoft's
   [SAC FAQ](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/smart-app-control-frequently-asked-questions)
   first.

PeekBuddy does not detect SAC and does not work around it.

## Mark of the Web

A file from the internet carries a *Mark of the Web* that makes SmartScreen
look at it. For an archive you extracted yourself, PowerShell can remove the
mark from one file:

```powershell
Unblock-File .\peekbuddy.exe
```

That tells Windows you trust this file; it removes a warning, not a risk. Do it
only after you verified the download, and only for this file. Never turn off
Microsoft Defender to run PeekBuddy.

## Defender and false positives

A new, unsigned executable is sometimes flagged by Microsoft Defender. Each
release is scanned before it is announced, and anything flagged is sent to
Microsoft for analysis. If Defender blocks your copy:

- check the hash against `SHA256SUMS` first (it is the published file);
- report it in an issue with the Defender message, so the report can be
  linked to Microsoft's;
- do not exclude folders on a hunch.

PeekBuddy keeps traits that look like malware out of the program: no hidden
autostart (it is opt-in and visible), no copy of itself, no obfuscation, no
network access.
