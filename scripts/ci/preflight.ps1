<#
.SYNOPSIS
    Fails fast when a hosted Windows runner cannot run PeekBuddy's UI and tray tests.
.DESCRIPTION
    Asserts an interactive console session (session id != 0), a visible input
    desktop (WSF_VISIBLE) and a running explorer.exe. Prints image version,
    Defender state, timezone, resolution and runner size. Spike #34; reused
    by #11, #27 and #71. Budget: <= 10 s.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PeekBuddyPreflight {
    [StructLayout(LayoutKind.Sequential)]
    struct USEROBJECTFLAGS { public int fInherit; public int fReserved; public uint dwFlags; }
    [DllImport("user32.dll", SetLastError = true)]
    static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool GetUserObjectInformationW(IntPtr obj, int index, ref USEROBJECTFLAGS info, int length, out int needed);
    [DllImport("user32.dll")]
    static extern bool CloseDesktop(IntPtr desk);
    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int index);
    public static long InputDesktopFlags() {
        IntPtr desk = OpenInputDesktop(0, false, 1 /* DESKTOP_READOBJECTS */);
        if (desk == IntPtr.Zero) { return -Marshal.GetLastWin32Error(); }
        try {
            var flags = new USEROBJECTFLAGS();
            int needed;
            if (!GetUserObjectInformationW(desk, 1 /* UOI_FLAGS */, ref flags, Marshal.SizeOf(flags), out needed)) {
                return -Marshal.GetLastWin32Error();
            }
            return flags.dwFlags;
        } finally { CloseDesktop(desk); }
    }
}
'@

$failures = [System.Collections.Generic.List[string]]::new()

$sessionId = (Get-Process -Id $PID).SessionId
if ($sessionId -eq 0) { $failures.Add('process runs in session 0 (no interactive desktop)') }

$flags = [PeekBuddyPreflight]::InputDesktopFlags()
if ($flags -lt 0) {
    $failures.Add("cannot open the input desktop (Win32 error $(-$flags))")
} elseif (($flags -band 1) -eq 0) {
    $failures.Add("input desktop is not visible (dwFlags=0x$('{0:x}' -f $flags))")
}

$explorer = @(Get-Process -Name explorer -ErrorAction SilentlyContinue)
if ($explorer.Count -eq 0) { $failures.Add('explorer.exe is not running (no tray)') }

$defender = 'unknown'
try {
    $mp = Get-MpComputerStatus
    $defender = "realtime=$($mp.RealTimeProtectionEnabled) tamper=$($mp.IsTamperProtected)"
} catch { $defender = 'unavailable' }

$os = Get-CimInstance -ClassName Win32_OperatingSystem
$info = [ordered]@{
    image        = "$env:ImageOS $env:ImageVersion"
    os           = "$($os.Caption) build $($os.BuildNumber)"
    architecture = $env:PROCESSOR_ARCHITECTURE
    cpus         = [Environment]::ProcessorCount
    memoryGiB    = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    session      = $sessionId
    desktopFlags = $flags
    explorer     = $explorer.Count
    defender     = $defender
    timezone     = (Get-TimeZone).Id
    resolution   = "$([PeekBuddyPreflight]::GetSystemMetrics(0))x$([PeekBuddyPreflight]::GetSystemMetrics(1))"
}
$info.GetEnumerator() | ForEach-Object { Write-Output ('{0,-13} {1}' -f $_.Key, $_.Value) }
Write-Output ('{0,-13} {1:N1} s' -f 'preflight', ((Get-Date) - $started).TotalSeconds)

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Output "::error title=preflight::$failure" }
    exit 1
}
