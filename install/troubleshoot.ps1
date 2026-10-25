# Checks every piece of API Home, prints OK / WARN / FAIL with what to do, and saves the report in LOGS.
#
#   TROUBLESHOOT.bat           check only (if forge isn't running it is started for the test and stopped again)
#   TROUBLESHOOT.bat -Fix      also offers the fixes: start the tray, stop whatever blocks the port
#   troubleshoot.ps1 -Quiet    no questions, no report window; exit code = number of FAILs (CI)
param(
    [string]$Dir = '',
    [switch]$Fix,
    [switch]$Quiet
)
$ErrorActionPreference = 'Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Dir) {
    $Dir = $here
    if (-not (Test-Path (Join-Path $Dir 'forge.exe')) -and $env:LOCALAPPDATA -and (Test-Path (Join-Path $env:LOCALAPPDATA 'Forge\forge.exe'))) {
        $Dir = Join-Path $env:LOCALAPPDATA 'Forge'
    }
}
$base = if ($env:FORGE_URL) { $env:FORGE_URL.TrimEnd('/') } else { 'http://127.0.0.1:8790' }
$port = 8790; if ($base -match ':(\d+)$') { $port = [int]$Matches[1] }
$lines = New-Object System.Collections.Generic.List[string]
$fails = 0
function Line($t) { Write-Host $t; $lines.Add($t) }
function Ok($t) { Line "  OK    $t" }
function Warn($t, $do) { Line "  WARN  $t"; if ($do) { Line "        -> $do" } }
function Fail($t, $do) { $script:fails++; Line "  FAIL  $t"; if ($do) { Line "        -> $do" } }
function Ask($q) { if ($Quiet -or -not $Fix) { return $false } return ((Read-Host "        $q [y/N]") -match '^[yY]') }
function Get-Json($path, $timeout = 10) {
    try { return Invoke-RestMethod -Uri "$base$path" -TimeoutSec $timeout } catch { return $null }
}

Line "API Home troubleshoot  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Line "install folder: $Dir"
Line "forge address:  $base"
Line ''

# 1. programs
Line '[programs]'
$forge = Join-Path $Dir 'forge.exe'; $tray = Join-Path $Dir 'forge-tray.exe'
if (Test-Path $forge) { Ok "forge.exe  $((Get-Item $forge).LastWriteTime)" } else { Fail 'forge.exe missing' 'run INSTALL.bat' }
if (Test-Path $tray) { Ok "forge-tray.exe  $((Get-Item $tray).LastWriteTime)" } else { Fail 'forge-tray.exe missing' 'run INSTALL.bat' }
$cfg = Join-Path $Dir 'config'
foreach ($f in 'providers.json', 'settings.json') {
    $p = Join-Path $cfg $f
    if (-not (Test-Path $p)) { Warn "config\$f missing (built-in defaults are used)" "run: `"$forge`" init `"$Dir`"" }
    else { try { $null = [IO.File]::ReadAllText($p).TrimStart([char]0xFEFF) | ConvertFrom-Json; Ok "config\$f" } catch { Fail "config\$f is not valid JSON: $($_.Exception.Message)" 'fix the file, or rename it and run forge init again' } }
}
Line ''

# 2. python (the pipeline's console.py and stations run on it)
Line '[python]'
$py = $null; $pyArgs = @()
if ($env:PYTHON) { $py = $env:PYTHON }
elseif (Get-Command py -ErrorAction SilentlyContinue) { $py = 'py'; $pyArgs = @('-3') }
elseif (Get-Command python3 -ErrorAction SilentlyContinue) { $py = 'python3' }
elseif (Get-Command python -ErrorAction SilentlyContinue) { $py = 'python' }
if ($py) {
    $v = & $py @pyArgs --version 2>&1
    if ($LASTEXITCODE -eq 0) { Ok "$py $($pyArgs -join ' ') -> $v" } else { Fail "$py does not run: $v" 'install Python 3 from python.org (tick "py launcher"), or set PYTHON to python.exe' }
} else { Fail 'no Python found (py, python3, python)' 'install Python 3 from python.org (tick "py launcher")' }
if (-not $env:PYTHON -and $py -and $py -ne 'py') { Warn "forge looks for 'py -3' on Windows; you have $py" "setx PYTHON `"$((Get-Command $py).Source)`"" }
Line ''

# 3. API sources (config\apis.json + PIPELINE_SYSTEM)
Line '[API sources]'
$reg = Join-Path $cfg 'apis.json'
$sources = @()
if (Test-Path $reg) {
    try { $sources = @((([IO.File]::ReadAllText($reg).TrimStart([char]0xFEFF)) | ConvertFrom-Json).sources | Where-Object { $_ }) }
    catch { Fail "config\apis.json is not valid JSON: $($_.Exception.Message)" 'fix or delete it, then add the folders again on the home page' }
}
if ($env:PIPELINE_SYSTEM) { $sources = @([pscustomobject]@{ name = 'PIPELINE_SYSTEM'; path = $env:PIPELINE_SYSTEM }) + $sources }
if (-not $sources.Count) { Fail 'no API folder registered' "INSTALL.bat with your pipeline folder, or add it on the home page ($base/)" }
foreach ($s in $sources) {
    $p = [string]$s.path
    if ($p -match '^https?://') { Ok "$($s.name): link $p"; continue }
    if (-not (Test-Path $p)) { Fail "$($s.name): not found: $p" 'fix the path on the home page (Remove, then Add)'; continue }
    $console = @((Join-Path $p '_system\engine\console.py'), (Join-Path $p 'engine\console.py')) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($console) {
        if (-not $py) { Fail "$($s.name): pipeline, but no Python to read it" ''; continue }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        Push-Location (Split-Path (Split-Path $console))
        $errFile = [IO.Path]::GetTempFileName()
        $out = & $py @pyArgs $console list 2> $errFile
        $code = $LASTEXITCODE
        $errText = (Get-Content $errFile -ErrorAction SilentlyContinue | Select-Object -Last 5) -join ' | '
        Remove-Item $errFile -ErrorAction SilentlyContinue
        Pop-Location
        try { $n = @((($out | Out-String) | ConvertFrom-Json).stations).Count } catch { $n = -1 }
        if ($code -eq 0 -and $n -ge 0) { Ok "$($s.name): pipeline, $n stations ($([int]$sw.Elapsed.TotalSeconds) s)  $p" }
        else { Fail "$($s.name): console.py list failed (exit $code)" $errText }
    } elseif ((Get-Item $p).PSIsContainer) {
        $n = @(Get-ChildItem $p -Recurse -Depth 3 -Include *.bat, *.html -File -ErrorAction SilentlyContinue).Count
        if ($n) { Ok "$($s.name): folder, $n .bat/.html files  $p" } else { Warn "$($s.name): folder has no .bat or .html (3 levels): $p" 'is this the right folder?' }
    } else { Ok "$($s.name): page $p" }
}
Line ''

# 4. forge itself
Line '[forge]'
$started = $null
$health = Get-Json '/health' 3
if (-not $health) {
    $listener = $null
    try { $listener = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction Stop | Select-Object -First 1 } catch {}
    if ($listener) {
        $owner = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
        Fail "port $port is taken by $($owner.Name) (pid $($listener.OwningProcess)), which is not answering as forge" "close it, or set FORGE_URL to another port (setx FORGE_URL http://127.0.0.1:8791)"
        if ($owner -and (Ask "stop $($owner.Name) (pid $($owner.Id))?")) { Stop-Process -Id $owner.Id -Force; Start-Sleep 1 }
    }
    if ((Test-Path $forge) -and -not (Get-Json '/health' 2)) {
        Line "  ..    forge not running: starting it for this test"
        New-Item -ItemType Directory -Force -Path (Join-Path $Dir 'LOGS') | Out-Null
        $started = Start-Process -FilePath $forge -ArgumentList @('--home', "`"$Dir`"", 'serve', '--port', "$port") -WorkingDirectory $Dir -PassThru -NoNewWindow `
            -RedirectStandardError (Join-Path $Dir 'LOGS\troubleshoot-forge.err.log') -RedirectStandardOutput (Join-Path $Dir 'LOGS\troubleshoot-forge.out.log')
        foreach ($i in 1..40) { if (Get-Json '/health' 2) { break }; Start-Sleep -Milliseconds 500 }
    }
    $health = Get-Json '/health' 3
}
if ($health) { Ok "forge answers at $base" } else {
    $err = Join-Path $Dir 'LOGS\troubleshoot-forge.err.log'
    $tail = if (Test-Path $err) { (Get-Content $err -Tail 5) -join ' | ' } else { '' }
    Fail "forge does not answer at $base" "see LOGS\forge-serve.log. $tail"
}
if ($health) {
    $apis = Get-Json '/api/apis' 90
    if ($apis) { foreach ($s in $apis.sources) { if ($s.error) { Warn "home page source $($s.name): $($s.error)" '' } } }
    try {
        $feed = Invoke-RestMethod -Uri "$base/api/stations" -TimeoutSec 120
        Ok "station feed: $(@($feed.stations).Count) stations"
        foreach ($e in @($feed.errors)) { if ($e) { Warn "$($e.source): $($e.error)" '' } }
    } catch {
        $msg = $_.ErrorDetails.Message; if (-not $msg) { $msg = $_.Exception.Message }
        Fail "station feed: $msg" 'register your pipeline API folder on the home page'
    }
    try { $null = Invoke-WebRequest -UseBasicParsing -Uri "$base/" -TimeoutSec 10; Ok "home page $base/" } catch { Fail "home page: $($_.Exception.Message)" '' }
    if (Test-Path $forge) {
        Line ''
        Line '[forge doctor]'
        & $forge --home $Dir doctor 2>&1 | ForEach-Object { Line "        $_" }
    }
}
if ($started) { Stop-Process -Id $started.Id -Force -ErrorAction SilentlyContinue; Line '  ..    test forge stopped' }
Line ''

# 5. tray, shortcuts, keys
Line '[taskbar, keys]'
$running = Get-Process forge-tray -ErrorAction SilentlyContinue
if ($running) { Ok "forge-tray running (pid $($running.Id -join ', '))" } else {
    Warn 'forge-tray is not running' 'start "API Home" from the Start menu, or LAUNCH.bat'
    if ((Test-Path $tray) -and (Ask 'start it now?')) { Start-Process -FilePath $tray -WorkingDirectory $Dir }
}
if ($env:APPDATA) {
    $lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'API Home.lnk'
    if (Test-Path $lnk) { Ok 'Start menu shortcut' } else { Warn 'no Start menu shortcut' 'run INSTALL.bat' }
}
foreach ($k in 'DEEPSEEK_API_KEY') {
    $set = [Environment]::GetEnvironmentVariable($k, 'User') -or [Environment]::GetEnvironmentVariable($k, 'Machine') -or (Get-Item "env:$k" -ErrorAction SilentlyContinue)
    if ($set) { Ok "$k is set (value not shown)" } else { Warn "$k is not set: model calls will fail" "setx $k `"sk-...`", then restart API Home" }
}
Line ''
Line $(if ($fails) { "$fails problem(s) found. Fixes are under each FAIL." } else { 'No problems found.' })

$logs = Join-Path $Dir 'LOGS'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$report = Join-Path $logs ("troubleshoot-{0}.txt" -f (Get-Date -Format 'yyyy-MM-dd_HHmmss'))
[IO.File]::WriteAllLines($report, $lines)
Write-Host "Report: $report"
if (-not $Quiet -and $env:OS -eq 'Windows_NT') { Start-Process notepad.exe $report }
exit $fails
