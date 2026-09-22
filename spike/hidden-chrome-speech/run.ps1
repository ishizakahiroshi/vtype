# 使い捨て実験。一時プロフィールの Chrome / Edge だけを起動して、終わったら消す。
# --remote-debugging-port は使わない。依頼者の普段のプロフィールと窓には触らない。
# 閉じる試験の観察中はプロセスを強制終了しない（finally の掃除はその後）。
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('offscreen', 'minimized', 'normal')]
    [string] $Mode,

    [Parameter(Mandatory = $true)]
    [ValidateSet('chrome', 'edge')]
    [string] $Browser,

    [ValidateSet('none', 'close', 'move', 'level')]
    [string] $Probe = 'none',

    [int] $DurationSeconds = 600,

    [string] $Tag = '',

    [string] $AudioFile = '',

    [switch] $PlayToSpeakers
)

$ErrorActionPreference = 'Stop'

if ($Browser -notin @('chrome', 'edge')) { throw "bad browser [$Browser]" }
if ($DurationSeconds -lt 1) { throw 'DurationSeconds must be >= 1' }

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if ($AudioFile) {
    $wav = $AudioFile
} else {
    $wav = Join-Path $here 'ja.wav'
}
if (-not (Test-Path -LiteralPath $wav)) { throw "missing WAV: $wav" }
$wav = [System.IO.Path]::GetFullPath($wav)
# SoundPlayer はファイルを開いたままにする。finally が消す一時コピーではなく、元の WAV を再生する。
$speakerWav = $wav

$suffix = if ($Tag) { "-$Tag" } else { '' }
$resultsPath = Join-Path $here ("results-{0}-{1}{2}.jsonl" -f $Browser, $Mode, $suffix)
$metaPath = Join-Path $here ("run-meta-{0}-{1}{2}.json" -f $Browser, $Mode, $suffix)
$earlyPng = Join-Path $here ("taskbar-{0}-{1}{2}-early.png" -f $Browser, $Mode, $suffix)
$latePng = Join-Path $here ("taskbar-{0}-{1}{2}-late.png" -f $Browser, $Mode, $suffix)

if (-not ('VtypeSpikeWin32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class VtypeSpikeWin32 {
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);

    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public const int SW_MINIMIZE = 6;
    public const uint WM_CLOSE = 0x0010;

    static HashSet<uint> Parse(string pidCsv) {
        var set = new HashSet<uint>();
        if (string.IsNullOrEmpty(pidCsv)) return set;
        foreach (var part in pidCsv.Split(',')) {
            uint v;
            if (uint.TryParse(part.Trim(), out v)) set.Add(v);
        }
        return set;
    }

    public static string[] ListWindows(string pidCsv) {
        var set = Parse(pidCsv);
        var found = new List<string>();
        EnumWindows((h, l) => {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (!set.Contains(pid) || !IsWindowVisible(h)) return true;
            var titleSb = new StringBuilder(512);
            GetWindowText(h, titleSb, titleSb.Capacity);
            var classSb = new StringBuilder(256);
            GetClassName(h, classSb, classSb.Capacity);
            RECT r;
            if (!GetWindowRect(h, out r)) return true;
            string title = titleSb.ToString().Replace("\t", " ").Replace("\r", " ").Replace("\n", " ");
            string cls = classSb.ToString().Replace("\t", " ");
            found.Add(string.Format(
                System.Globalization.CultureInfo.InvariantCulture,
                "{0}\t{1}\t{2}\t{3}\t{4}\t{5}\t{6}\t{7}\t{8}",
                h.ToInt64(), pid, IsIconic(h) ? 1 : 0, r.Left, r.Top, r.Right, r.Bottom, cls, title));
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    public static int[] TrayRect() {
        IntPtr h = FindWindow("Shell_TrayWnd", null);
        if (h == IntPtr.Zero) return null;
        RECT r;
        if (!GetWindowRect(h, out r)) return null;
        return new int[] { r.Left, r.Top, r.Right, r.Bottom };
    }
}
'@
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

function Format-Arg([string] $Value) {
    if ($Value -match '[\s"]') {
        return '"' + ($Value.Replace('"', '\"')) + '"'
    }
    return $Value
}

function Get-BrowserExe([string] $Name) {
    $exe = if ($Name -eq 'edge') { 'msedge.exe' } else { 'chrome.exe' }
    $key = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\$exe"
    $prop = Get-ItemProperty -Path $key
    $path = [string] $prop.'(default)'
    if (-not $path -or -not (Test-Path -LiteralPath $path)) {
        throw "App Paths entry missing for $exe"
    }
    return [System.IO.Path]::GetFullPath($path)
}

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return ([System.Net.IPEndPoint] $listener.LocalEndpoint).Port
    } finally {
        $listener.Stop()
    }
}

function Test-PortOpen([int] $Port) {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $iar = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
        if (-not $iar.AsyncWaitHandle.WaitOne(300)) { return $false }
        $client.EndConnect($iar)
        return $true
    } catch {
        return $false
    } finally {
        $client.Dispose()
    }
}

function Get-ProfileProcesses([string] $UserData) {
    @(Get-CimInstance Win32_Process | Where-Object {
        $_.CommandLine -and $_.CommandLine.Contains($UserData)
    })
}

function Convert-WindowRows([string[]] $Lines) {
    $rows = @()
    foreach ($line in $Lines) {
        if (-not $line) { continue }
        $p = $line -split "`t", 9
        if ($p.Count -lt 9) { continue }
        $left = [int] $p[3]; $top = [int] $p[4]; $right = [int] $p[5]; $bottom = [int] $p[6]
        $rows += [pscustomobject]@{
            hwnd      = [int64] $p[0]
            pid       = [int] $p[1]
            iconic    = ([int] $p[2] -eq 1)
            left      = $left
            top       = $top
            right     = $right
            bottom    = $bottom
            className = $p[7]
            title     = $p[8]
            onScreen  = (Test-RectOnScreen $left $top $right $bottom)
        }
    }
    return $rows
}

function Test-RectOnScreen([int] $Left, [int] $Top, [int] $Right, [int] $Bottom) {
    foreach ($screen in [System.Windows.Forms.Screen]::AllScreens) {
        $b = $screen.Bounds
        $outside = ($Right -le $b.X) -or ($Left -ge ($b.X + $b.Width)) -or ($Bottom -le $b.Y) -or ($Top -ge ($b.Y + $b.Height))
        if (-not $outside) { return $true }
    }
    return $false
}

function Get-AppWindows($Rows) {
    @($Rows | Where-Object {
        $_.className -eq 'Chrome_WidgetWin_1' -and (
            $_.title -or (($_.right - $_.left) -ge 200 -and ($_.bottom - $_.top) -ge 150)
        )
    })
}

function Get-TaskbarHit {
    try {
        Add-Type -AssemblyName UIAutomationClient -ErrorAction Stop
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $cond = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ClassNameProperty,
            'Shell_TrayWnd')
        $tray = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
        if (-not $tray) { return 'tray-not-found' }
        $all = $tray.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition)
        $hits = New-Object System.Collections.Generic.List[string]
        foreach ($el in $all) {
            $name = $el.Current.Name
            if ($name -and $name.Contains('vtype-spike-speech')) {
                $hits.Add($name) | Out-Null
            }
        }
        if ($hits.Count -gt 0) { return 'yes' }
        return 'no'
    } catch {
        return ('uia-error:' + $_.Exception.Message)
    }
}

function Save-TaskbarPng([string] $Path) {
    $rect = [VtypeSpikeWin32]::TrayRect()
    if ($null -eq $rect) { return 'tray-not-found' }
    $left = $rect[0]; $top = $rect[1]; $w = $rect[2] - $rect[0]; $h = $rect[3] - $rect[1]
    if ($w -le 0 -or $h -le 0) { return 'bad-rect' }
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    try {
        $g.CopyFromScreen($left, $top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
        return 'saved'
    } finally {
        $g.Dispose()
        $bmp.Dispose()
    }
}

function Get-PidCsv($Procs) {
    (@($Procs | ForEach-Object { [string] $_.ProcessId })) -join ','
}

function Get-Sample([string] $UserData, [datetime] $Origin, [switch] $SkipTaskbar) {
    $procs = @(Get-ProfileProcesses $UserData)
    $csv = Get-PidCsv $procs
    $windows = @(Convert-WindowRows ([VtypeSpikeWin32]::ListWindows($csv)))
    $handles = @()
    foreach ($proc in $procs) {
        $gp = Get-Process -Id $proc.ProcessId -ErrorAction SilentlyContinue
        if (-not $gp) { continue }
        $handles += [pscustomobject]@{
            pid              = [int] $proc.ProcessId
            processName      = [string] $gp.ProcessName
            mainWindowHandle = [int64] $gp.MainWindowHandle
        }
    }
    $app = @(Get-AppWindows $windows)
    [pscustomobject]@{
        at              = (Get-Date).ToString('o')
        elapsedSec      = [math]::Round(((Get-Date) - $Origin).TotalSeconds, 1)
        processCount    = $procs.Count
        mainWindowCount = @($handles | Where-Object { $_.mainWindowHandle -ne 0 }).Count
        handles         = $handles
        windows         = $windows
        appWindows      = $app
        appOnScreen     = @($app | Where-Object { $_.onScreen -and -not $_.iconic }).Count
        appIconic       = @($app | Where-Object { $_.iconic }).Count
        taskbar         = $(if ($SkipTaskbar) { 'skipped' } else { Get-TaskbarHit })
    }
}

function Invoke-MinimizeOurs($Procs) {
    $csv = Get-PidCsv $Procs
    $rows = @(Get-AppWindows (Convert-WindowRows ([VtypeSpikeWin32]::ListWindows($csv))))
    $n = 0
    foreach ($row in $rows) {
        if ([VtypeSpikeWin32]::ShowWindow([IntPtr]::new($row.hwnd), [VtypeSpikeWin32]::SW_MINIMIZE)) {
            $n++
        } else {
            # ShowWindow returns false when the window was previously hidden. Still a successful call.
            $n++
        }
    }
    return $n
}

function Invoke-CloseOurs($Procs) {
    $csv = Get-PidCsv $Procs
    $rows = @(Get-AppWindows (Convert-WindowRows ([VtypeSpikeWin32]::ListWindows($csv))))
    foreach ($row in $rows) {
        [VtypeSpikeWin32]::PostMessage([IntPtr]::new($row.hwnd), [VtypeSpikeWin32]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    }
    return $rows.Count
}

$browserExe = Get-BrowserExe $Browser
$port = Get-FreePort
$userData = Join-Path $env:TEMP ("vtype-spike-hidden-chrome-" + [guid]::NewGuid().ToString('n'))
$userData = [System.IO.Path]::GetFullPath($userData)
if ($userData -notlike '*vtype-spike-hidden-chrome-*') { throw 'refusing to use a path outside the spike prefix' }
if ($userData -match 'Google\\Chrome\\User Data' -or $userData -match 'Microsoft\\Edge\\User Data') {
    throw 'refusing to touch the everyday browser profile'
}
New-Item -ItemType Directory -Path $userData | Out-Null
# The audio service sandbox often cannot read the repo. Keep the capture file inside the temp profile.
$captureWav = Join-Path $userData 'capture.wav'
Copy-Item -LiteralPath $wav -Destination $captureWav -Force
$wav = $captureWav

$node = (Get-Command node -ErrorAction Stop).Source
if (Test-Path -LiteralPath $resultsPath) { Remove-Item -LiteralPath $resultsPath -Force }
New-Item -ItemType File -Path $resultsPath | Out-Null

$nodeArgs = @(
    (Format-Arg (Join-Path $here 'server.mjs'))
    [string] $port
    (Format-Arg $resultsPath)
) -join ' '

$serverPsi = New-Object System.Diagnostics.ProcessStartInfo
$serverPsi.FileName = $node
$serverPsi.Arguments = $nodeArgs
$serverPsi.WorkingDirectory = $here
$serverPsi.UseShellExecute = $false
$serverPsi.CreateNoWindow = $true
$server = New-Object System.Diagnostics.Process
$server.StartInfo = $serverPsi

$browserProc = $null
$origin = Get-Date
$samples = New-Object System.Collections.Generic.List[object]
$shotEarly = $null
$shotLate = $null
$minimizeCalls = 0
$startMinimizedUsed = $false
$runError = $null
$measureEnd = $null
$player = $null

try {
    if ($PlayToSpeakers) {
        Add-Type -AssemblyName System.Windows.Extensions
        $player = New-Object System.Media.SoundPlayer $speakerWav
        $player.Load()
        $player.PlayLooping()
        Write-Output ("play-looping " + $speakerWav)
    }
    if (-not $server.Start()) { throw 'failed to start server.mjs' }
    $opened = $false
    $waitUntil = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $waitUntil) {
        if ($server.HasExited) { throw "server.mjs exited early (code $($server.ExitCode))" }
        if (Test-PortOpen $port) { $opened = $true; break }
        Start-Sleep -Milliseconds 200
    }
    if (-not $opened) { throw "server.mjs did not listen on 127.0.0.1:$port" }

    $browserArgs = @(
        "--user-data-dir=$userData"
        "--app=http://127.0.0.1:${port}/?probe=$Probe"
        '--no-first-run'
        '--no-default-browser-check'
        '--use-fake-device-for-media-stream'
        "--use-file-for-fake-audio-capture=$wav"
        '--use-fake-ui-for-media-stream'
    )
    if ($Mode -eq 'offscreen') {
        $browserArgs += '--window-position=-32000,-32000'
        $browserArgs += '--window-size=400,300'
    }
    if ($Mode -eq 'minimized') {
        $browserArgs += '--start-minimized'
        $startMinimizedUsed = $true
    }
    $argString = ($browserArgs | ForEach-Object { Format-Arg $_ }) -join ' '
    Write-Output ("browser-args " + $argString)

    $browserPsi = New-Object System.Diagnostics.ProcessStartInfo
    $browserPsi.FileName = $browserExe
    $browserPsi.Arguments = $argString
    $browserPsi.UseShellExecute = $false
    $browserProc = New-Object System.Diagnostics.Process
    $browserProc.StartInfo = $browserPsi
    $origin = Get-Date
    if (-not $browserProc.Start()) { throw 'failed to start the browser' }

    $seen = $false
    $appearUntil = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $appearUntil) {
        $procs = @(Get-ProfileProcesses $userData)
        if ($procs.Count -gt 0) { $seen = $true; break }
        Start-Sleep -Milliseconds 300
    }
    if (-not $seen) { throw 'browser process with the temp profile did not appear' }

    if ($Mode -eq 'minimized' -and $Probe -eq 'none') {
        Start-Sleep -Seconds 2
        $minimizeCalls += Invoke-MinimizeOurs (Get-ProfileProcesses $userData)
        Start-Sleep -Seconds 1
        # --start-minimized often does nothing for --app. If a window is still restored, minimize that hwnd only.
        $check = Get-Sample $userData $origin
        $restored = @($check.appWindows | Where-Object { -not $_.iconic })
        if ($restored.Count -gt 0) {
            $minimizeCalls += Invoke-MinimizeOurs (Get-ProfileProcesses $userData)
        }
    }

    if ($Probe -eq 'none') {
        $marks = New-Object System.Collections.Generic.List[int]
        $marks.Add([Math]::Min(8, $DurationSeconds)) | Out-Null
        if ($DurationSeconds -ge 120) { $marks.Add([int] ($DurationSeconds / 2)) | Out-Null }
        if ($DurationSeconds -ge 30) { $marks.Add([Math]::Max(1, $DurationSeconds - 5)) | Out-Null }
        $deadline = $origin.AddSeconds($DurationSeconds)
        foreach ($mark in ($marks | Sort-Object -Unique)) {
            $target = $origin.AddSeconds($mark)
            while ((Get-Date) -lt $target -and (Get-Date) -lt $deadline) {
                $remainMs = [int] (($target - (Get-Date)).TotalMilliseconds)
                if ($remainMs -gt 200) { Start-Sleep -Milliseconds ([Math]::Min(5000, $remainMs)) }
                else { break }
            }
            if ($Mode -eq 'minimized') {
                $minimizeCalls += Invoke-MinimizeOurs (Get-ProfileProcesses $userData)
            }
            $sample = Get-Sample $userData $origin
            $samples.Add($sample) | Out-Null
            $browserState = if ($browserProc -and -not $browserProc.HasExited) { 'alive' } elseif ($browserProc) { "dead:$($browserProc.ExitCode)" } else { 'none' }
            Write-Output ("sample elapsed=$($sample.elapsedSec) procs=$($sample.processCount) mainWindows=$($sample.mainWindowCount) onScreen=$($sample.appOnScreen) iconic=$($sample.appIconic) taskbar=$($sample.taskbar) browser=$browserState")
            if (-not $shotEarly) {
                try { $shotEarly = Save-TaskbarPng $earlyPng } catch { $shotEarly = $_.Exception.Message }
            }
        }
        while ((Get-Date) -lt $deadline) {
            $remainMs = [int] (($deadline - (Get-Date)).TotalMilliseconds)
            if ($remainMs -le 200) { break }
            Start-Sleep -Milliseconds ([Math]::Min(5000, $remainMs))
        }
    } else {
        $deadline = $origin.AddSeconds($DurationSeconds)
        $sawAction = $false
        while ((Get-Date) -lt $deadline) {
            $sample = Get-Sample $userData $origin -SkipTaskbar
            $samples.Add($sample) | Out-Null
            Write-Output ("probe-sample elapsed=$($sample.elapsedSec) procs=$($sample.processCount) mainWindows=$($sample.mainWindowCount) onScreen=$($sample.appOnScreen) iconic=$($sample.appIconic) wins=$($sample.windows.Count)")
            if (-not $shotEarly) {
                try { $shotEarly = Save-TaskbarPng $earlyPng } catch { $shotEarly = $_.Exception.Message }
            }
            $hit = $false
            if (Test-Path -LiteralPath $resultsPath) {
                $text = [System.IO.File]::ReadAllText($resultsPath)
                if ($Probe -eq 'move' -and $text.Contains('"move-after"')) { $hit = $true }
                if ($Probe -eq 'close' -and $text.Contains('"close-before"') -and $sample.appWindows.Count -eq 0) { $hit = $true }
            }
            if ($hit -and -not $sawAction) { $sawAction = $true }
            if ($hit) {
                Start-Sleep -Seconds 1
                $samples.Add((Get-Sample $userData $origin -SkipTaskbar)) | Out-Null
                break
            }
            Start-Sleep -Seconds 1
        }
    }

    $measureEnd = (Get-Date).ToString('o')
    if ($Mode -eq 'minimized' -and $Probe -eq 'none') {
        $minimizeCalls += Invoke-MinimizeOurs (Get-ProfileProcesses $userData)
    }
    $finalSample = Get-Sample $userData $origin
    $samples.Add($finalSample) | Out-Null
    $browserState = if ($browserProc -and -not $browserProc.HasExited) { 'alive' } elseif ($browserProc) { "dead:$($browserProc.ExitCode)" } else { 'none' }
    Write-Output ("final elapsed=$($finalSample.elapsedSec) procs=$($finalSample.processCount) mainWindows=$($finalSample.mainWindowCount) onScreen=$($finalSample.appOnScreen) iconic=$($finalSample.appIconic) taskbar=$($finalSample.taskbar) browser=$browserState")
    try { $shotLate = Save-TaskbarPng $latePng } catch { $shotLate = $_.Exception.Message }
} catch {
    $runError = $_.Exception.Message
    Write-Output ("ERROR " + $runError)
    throw
} finally {
    if ($null -ne $player) {
        try {
            $player.Stop()
            $player.Dispose()
        } catch {
            Write-Output ("CLEANUP-PLAYER " + $_.Exception.Message)
        }
        $player = $null
    }
    $closedWindows = 0
    $remainingBeforeKill = 0
    $killed = @()
    $dirRemoved = $false
    try {
        $procs = @(Get-ProfileProcesses $userData)
        $closedWindows = Invoke-CloseOurs $procs
        $waitKill = (Get-Date).AddSeconds(8)
        while ((Get-Date) -lt $waitKill) {
            $procs = @(Get-ProfileProcesses $userData)
            if ($procs.Count -eq 0) { break }
            Start-Sleep -Milliseconds 400
        }
        $procs = @(Get-ProfileProcesses $userData)
        $remainingBeforeKill = $procs.Count
        foreach ($proc in $procs) {
            try {
                Stop-Process -Id $proc.ProcessId -Force -ErrorAction Stop
                $killed += [int] $proc.ProcessId
            } catch {
                # already exited
            }
        }
        $waitGone = (Get-Date).AddSeconds(5)
        while ((Get-Date) -lt $waitGone) {
            if (@(Get-ProfileProcesses $userData).Count -eq 0) { break }
            Start-Sleep -Milliseconds 300
        }
    } catch {
        Write-Output ("CLEANUP-BROWSER " + $_.Exception.Message)
    }
    try {
        if ($server -and -not $server.HasExited) {
            Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        }
    } catch {
        Write-Output ("CLEANUP-SERVER " + $_.Exception.Message)
    }
    for ($i = 0; $i -lt 10; $i++) {
        try {
            if (Test-Path -LiteralPath $userData) {
                Remove-Item -LiteralPath $userData -Recurse -Force -ErrorAction Stop
            }
            if (-not (Test-Path -LiteralPath $userData)) { $dirRemoved = $true; break }
        } catch {
            Start-Sleep -Seconds 1
        }
    }
    $meta = [ordered]@{
        mode                = $Mode
        browser             = $Browser
        probe               = $Probe
        tag                 = $Tag
        durationSeconds     = $DurationSeconds
        port                = $port
        browserExe          = $browserExe
        wav                 = $wav
        playToSpeakers      = [bool] $PlayToSpeakers
        speakerWav          = $(if ($PlayToSpeakers) { $speakerWav } else { $null })
        userData            = $userData
        userDataRemoved     = $dirRemoved
        resultsPath         = $resultsPath
        startedAt           = $origin.ToString('o')
        measureEnd          = $measureEnd
        startMinimizedUsed  = $startMinimizedUsed
        minimizeCalls       = $minimizeCalls
        shotEarly           = $shotEarly
        shotLate            = $shotLate
        earlyPng            = $earlyPng
        latePng             = $latePng
        closedWindows       = $closedWindows
        remainingBeforeKill = $remainingBeforeKill
        killedPids          = $killed
        error               = $runError
        samples             = @($samples.ToArray())
    }
    # Piping an ordered dictionary enumerates entries and ConvertTo-Json then fails.
    $json = ConvertTo-Json -InputObject $meta -Depth 8
    Set-Content -LiteralPath $metaPath -Value $json -Encoding utf8
    Write-Output ("META " + $metaPath)
    Write-Output ("RESULTS " + $resultsPath)
    Write-Output ("USERDATA_REMOVED " + $dirRemoved)
    Write-Output ("KILLED " + ($killed -join ','))
}
