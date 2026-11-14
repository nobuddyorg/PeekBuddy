<#
.SYNOPSIS
    Takes the screenshots of the README and the docs from a real run.
.DESCRIPTION
    Starts an engine on a temporary data directory that watches the demo
    folder C:\Home, makes a few dozen file changes with ordinary names, opens
    the viewer on that directory (light and dark theme, and the welcome
    screen of a first run) and captures the window's client area from the
    screen. Needs a desktop session, no elevation and no network; ends every
    process it started and removes its temporary files and C:\Home (it stops
    when C:\Home already exists). The images go to -OutDir (default
    docs\assets), named screenshot-<what>-<theme>.png.

    The capture copies the screen, so close or move other windows over the
    spot first: the viewer is kept on top at 80,60 with 1280 x 800 pixels.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
if (-not $OutDir) { $OutDir = Join-Path (Split-Path -Parent $here) 'docs\assets' }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$demo = 'C:\Home'
if (Test-Path $demo) { throw "$demo exists; this script uses and removes it" }

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder t, int n);
    // The visible top-level windows of a process whose title starts with a prefix.
    public static System.Collections.Generic.List<IntPtr> Windows(uint pid, string prefix) {
        var found = new System.Collections.Generic.List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) {
                var t = new System.Text.StringBuilder(256);
                GetWindowText(h, t, 256);
                if (t.ToString().StartsWith(prefix)) found.Add(h);
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
'@

# Pixels, not scaled units: the sizes below are what the window really has.
[Win]::SetProcessDPIAware() | Out-Null

function Wait-Window([System.Diagnostics.Process]$Process, [int]$Seconds = 90) {
    # The window titled "PeekBuddy [hash]" (an isolated viewer): the process
    # has other windows (the first one MainWindowHandle names is not it).
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $Process.Refresh()
        if ($Process.HasExited) { throw 'the viewer exited before its window appeared' }
        $windows = [Win]::Windows([uint32]$Process.Id, 'PeekBuddy [')
        if ($windows.Count -gt 0) { return $windows[0] }
        Start-Sleep -Milliseconds 200
    }
    throw 'no viewer window'
}

function Save-Window([IntPtr]$Handle, [string]$Path) {
    [Win]::ShowWindow($Handle, 9) | Out-Null
    # The viewer restores its own size and place in its first frames; ask until
    # it keeps the size, and keep it on top so the screen shows nothing else.
    $rect = New-Object Win+RECT
    $placed = $false
    for ($i = 0; $i -lt 30 -and -not $placed; $i++) {
        [Win]::MoveWindow($Handle, 80, 60, 1280, 800, $true) | Out-Null
        [Win]::SetWindowPos($Handle, [IntPtr](-1), 80, 60, 1280, 800, 0x0040) | Out-Null
        [Win]::SetForegroundWindow($Handle) | Out-Null
        Start-Sleep -Milliseconds 800
        [Win]::GetWindowRect($Handle, [ref]$rect) | Out-Null
        $placed = ([math]::Abs($rect.Left - 80) -le 12 -and [math]::Abs($rect.Top - 60) -le 12 -and ($rect.Right - $rect.Left) -ge 1270 -and ($rect.Bottom - $rect.Top) -ge 790)
    }
    # Once more after a pause: the viewer must keep what it was given.
    Start-Sleep -Seconds 4
    [Win]::GetWindowRect($Handle, [ref]$rect) | Out-Null
    if ([math]::Abs($rect.Left - 80) -gt 12 -or [math]::Abs($rect.Top - 60) -gt 12 -or ($rect.Right - $rect.Left) -lt 1270) {
        throw "the viewer did not keep its place: $($rect.Left),$($rect.Top) $($rect.Right - $rect.Left) x $($rect.Bottom - $rect.Top)"
    }
    # The client area only: no title bar (its text holds the hash of the data
    # directory) and no border.
    $client = New-Object Win+RECT
    [Win]::GetClientRect($Handle, [ref]$client) | Out-Null
    $origin = New-Object Win+POINT
    [Win]::ClientToScreen($Handle, [ref]$origin) | Out-Null
    $width = $client.Right
    $height = $client.Bottom
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    # PrintWindow asks the window to paint itself (client area only, full
    # content): nothing else on the desktop can get into the picture.
    $hdc = $graphics.GetHdc()
    $ok = [Win]::PrintWindow($Handle, $hdc, 3)
    $graphics.ReleaseHdc($hdc)
    if (-not $ok) { throw 'PrintWindow failed' }
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    Write-Output "wrote $Path ($width x $height)"
}

function New-DemoFolder {
    foreach ($dir in 'Documents\Taxes', 'Documents\Letters', 'Photos\2026-summer', 'Projects\garden-planner\src', 'Downloads') {
        New-Item -ItemType Directory -Force (Join-Path $demo $dir) | Out-Null
    }
}

function Invoke-DemoChanges {
    foreach ($i in 1..6) {
        Set-Content -Path (Join-Path $demo "Projects\garden-planner\build-$i.log") -Value "log $i" -Encoding ascii
        Start-Sleep -Milliseconds 100
    }
    $files = @(
        'Documents\Taxes\tax-return-2025.pdf', 'Documents\Taxes\receipts-march.xlsx', 'Documents\Letters\landlord-notice.docx',
        'Documents\Letters\insurance-claim.docx', 'Photos\2026-summer\IMG_0412.jpg', 'Photos\2026-summer\IMG_0413.jpg',
        'Photos\2026-summer\IMG_0414.jpg', 'Photos\2026-summer\beach-panorama.jpg', 'Projects\garden-planner\src\main.rs',
        'Projects\garden-planner\src\beds.rs', 'Projects\garden-planner\Cargo.toml', 'Projects\garden-planner\README.md',
        'Downloads\setup-installer.exe', 'Downloads\invoice-4711.pdf', 'Downloads\recipe-ideas.txt'
    )
    foreach ($f in $files) {
        Set-Content -Path (Join-Path $demo $f) -Value ('demo ' * 40) -Encoding ascii
        Start-Sleep -Milliseconds 400
    }
    foreach ($f in $files | Select-Object -First 9) {
        Add-Content -Path (Join-Path $demo $f) -Value 'edited' -Encoding ascii
        Start-Sleep -Milliseconds 350
    }
    Rename-Item (Join-Path $demo 'Downloads\recipe-ideas.txt') 'recipes-final.txt'
    Start-Sleep -Milliseconds 400
    Rename-Item (Join-Path $demo 'Photos\2026-summer\beach-panorama.jpg') 'beach-panorama-edited.jpg'
    Start-Sleep -Milliseconds 400
    Remove-Item (Join-Path $demo 'Downloads\setup-installer.exe')
    Start-Sleep -Milliseconds 400
    Remove-Item (Join-Path $demo 'Documents\Letters\insurance-claim.docx')
}

$work = Join-Path ([IO.Path]::GetTempPath()) "screenshots-$([guid]::NewGuid())"
New-Item -ItemType Directory $work | Out-Null
$started = New-Object System.Collections.Generic.List[System.Diagnostics.Process]
try {
    foreach ($theme in 'light', 'dark') {
        New-DemoFolder
        $data = Join-Path $work "data-$theme"
        New-Item -ItemType Directory $data | Out-Null
        $toml = "version = 1`n`n[general]`ntheme = '$theme'`nfirst_run = false`n`n[[watch]]`nid = 'home-demo'`npath = '$demo'`n"
        [IO.File]::WriteAllText((Join-Path $data 'settings.toml'), $toml, (New-Object System.Text.UTF8Encoding $false))
        $engine = Start-Process -FilePath $Exe -ArgumentList @('--headless', '--data-dir', "`"$data`"") -PassThru -WindowStyle Hidden
        $started.Add($engine)
        Start-Sleep -Seconds 6
        Invoke-DemoChanges
        Start-Sleep -Seconds 4
        $viewer = Start-Process -FilePath $Exe -ArgumentList @('--viewer', '--data-dir', "`"$data`"") -PassThru
        $started.Add($viewer)
        $handle = Wait-Window $viewer
        Start-Sleep -Seconds 3
        Save-Window $handle (Join-Path $OutDir "screenshot-log-$theme.png")
        Stop-Process -Id $viewer.Id -Force -ErrorAction SilentlyContinue
        Stop-Process -Id $engine.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
        Remove-Item -Recurse -Force $demo -ErrorAction SilentlyContinue
    }
    # The welcome screen of a first run.
    $data = Join-Path $work 'data-first-run'
    New-Item -ItemType Directory $data | Out-Null
    $viewer = Start-Process -FilePath $Exe -ArgumentList @('--viewer', '--data-dir', "`"$data`"") -PassThru
    $started.Add($viewer)
    $handle = Wait-Window $viewer
    Start-Sleep -Seconds 3
    Save-Window $handle (Join-Path $OutDir 'screenshot-welcome-light.png')
} finally {
    foreach ($p in $started) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $demo -ErrorAction SilentlyContinue
}
