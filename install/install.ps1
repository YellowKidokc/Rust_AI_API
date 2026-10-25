# Installs forge + the taskbar menu (forge-tray) for this Windows user. Run INSTALL.bat (it calls this).
#
#   INSTALL.bat                                   asks only what it can't find
#   install.ps1 -Pipeline "D:\GitHub\pipeline-workflows\API" -Startup
#   install.ps1 -Dest D:\Forge -Quiet -NoShortcuts -NoLaunch      (no questions; used by CI)
#
# What it does (nothing outside -Dest and your shortcuts is touched; config you already have is never overwritten):
#   1. finds forge.exe and forge-tray.exe (next to this script, or ..\target\release; builds them with cargo if needed)
#   2. copies them and LAUNCH / TROUBLESHOOT / UNINSTALL into -Dest (default %LOCALAPPDATA%\Forge)
#   3. forge init -Dest  (config\, templates\, INBOX\, OUTBOX\; existing files are kept)
#   4. registers your pipeline API folder in config\apis.json (the home page lists it; add more there)
#   5. Start menu + desktop shortcut "API Home"; with -Startup also starts with Windows
#   6. starts the tray (it starts forge by itself: no port to set)
param(
    [string]$Dest = (Join-Path $env:LOCALAPPDATA 'Forge'),
    [string]$Pipeline = '',
    [switch]$Startup,
    [switch]$NoShortcuts,
    [switch]$NoLaunch,
    [switch]$Quiet
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
function Say($t) { Write-Host $t }
function Ask($q) { if ($Quiet) { return '' } return (Read-Host $q) }
function Write-Utf8($path, $text) { [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false)) }

Say ''
Say '  Forge / API Home installer'
Say "  install folder: $Dest"
Say ''

# 1. the two programs
$src = $null
foreach ($d in @($here, (Join-Path $here '..\target\release'), (Join-Path $here 'target\release'))) {
    if ((Test-Path (Join-Path $d 'forge.exe')) -and (Test-Path (Join-Path $d 'forge-tray.exe'))) { $src = (Resolve-Path $d).Path; break }
}
if (-not $src) {
    $repo = Join-Path $here '..'
    if ((Test-Path (Join-Path $repo 'Cargo.toml')) -and (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Say '  forge.exe not built yet: cargo build --release (a few minutes the first time) ...'
        Push-Location $repo
        try { & cargo build --release; if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed (see above)' } } finally { Pop-Location }
        $src = (Resolve-Path (Join-Path $repo 'target\release')).Path
    } else {
        throw ("forge.exe / forge-tray.exe not found next to this script or in ..\target\release, and cargo is not installed.`n" +
               "  Download the CI artifact 'forge-windows-x64' (GitHub > Actions > latest green run), unzip it, and run INSTALL.bat from there.")
    }
}
Say "  programs from: $src"

# stop a running copy so its files can be replaced
Get-Process forge-tray, forge -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($Dest, [StringComparison]::OrdinalIgnoreCase) } |
    ForEach-Object { Say "  stopping running $($_.Name) ($($_.Id))"; Stop-Process -Id $_.Id -Force }
Start-Sleep -Milliseconds 500

# 2. copy
New-Item -ItemType Directory -Force -Path $Dest, (Join-Path $Dest 'LOGS') | Out-Null
Copy-Item (Join-Path $src 'forge.exe'), (Join-Path $src 'forge-tray.exe') -Destination $Dest -Force
foreach ($f in 'LAUNCH.bat', 'TROUBLESHOOT.bat', 'troubleshoot.ps1', 'UNINSTALL.bat', 'uninstall.ps1') {
    $p = Join-Path $here $f
    if (Test-Path $p) { Copy-Item $p -Destination $Dest -Force }
}
$forge = Join-Path $Dest 'forge.exe'

# 3. config (forge init never overwrites)
& $forge init $Dest | Out-Null
if ($LASTEXITCODE -ne 0) { throw "forge init $Dest failed" }

# 4. the pipeline API folder
function Test-Pipeline($p) {
    if (-not $p) { return $false }
    return (Test-Path (Join-Path $p '_system\engine\console.py')) -or (Test-Path (Join-Path $p 'engine\console.py'))
}
if (-not $Pipeline) {
    $guesses = @()
    foreach ($drive in 'D:', 'C:', 'E:') { $guesses += "$drive\GitHub\pipeline-workflows\API" }
    $guesses += (Join-Path $env:USERPROFILE 'Documents\GitHub\pipeline-workflows\API')
    $guesses += (Join-Path $env:USERPROFILE 'GitHub\pipeline-workflows\API')
    $Pipeline = $guesses | Where-Object { Test-Pipeline $_ } | Select-Object -First 1
}
while (-not (Test-Pipeline $Pipeline)) {
    if ($Pipeline) { Say "  not a pipeline API folder (no _system\engine\console.py): $Pipeline" }
    $Pipeline = (Ask '  Pipeline API folder (e.g. D:\GitHub\pipeline-workflows\API), Enter to skip').Trim('"', ' ')
    if (-not $Pipeline) { break }
}
$reg = Join-Path $Dest 'config\apis.json'
$sources = @()
if (Test-Path $reg) {
    $raw = [IO.File]::ReadAllText($reg).TrimStart([char]0xFEFF)
    if ($raw.Trim()) { $sources = @((ConvertFrom-Json $raw).sources | Where-Object { $_ }) }
}
if ($Pipeline) {
    $Pipeline = (Resolve-Path $Pipeline).Path
    if (-not ($sources | Where-Object { $_.path -eq $Pipeline })) {
        $sources += [pscustomobject]@{ name = 'Pipeline workflows'; group = 'Pipeline'; path = $Pipeline }
        Say "  registered pipeline: $Pipeline"
    } else { Say "  pipeline already registered: $Pipeline" }
} else {
    Say '  no pipeline registered: add your API folders on the home page later.'
}
Write-Utf8 $reg ((@{ sources = @($sources) } | ConvertTo-Json -Depth 5) + "`n")

# 5. shortcuts
$tray = Join-Path $Dest 'forge-tray.exe'
function New-Shortcut($path, $target) {
    $sh = New-Object -ComObject WScript.Shell
    $l = $sh.CreateShortcut($path); $l.TargetPath = $target; $l.WorkingDirectory = $Dest
    $l.Description = 'API Home: pipeline stations in the taskbar'; $l.Save()
}
if (-not $NoShortcuts) {
    $programs = [Environment]::GetFolderPath('Programs')
    New-Shortcut (Join-Path $programs 'API Home.lnk') $tray
    New-Shortcut (Join-Path ([Environment]::GetFolderPath('Desktop')) 'API Home.lnk') $tray
    Say '  shortcuts: Start menu + desktop "API Home"'
    if (-not $Startup -and -not $Quiet) { $Startup = (Ask '  Start API Home with Windows? [y/N]') -match '^[yY]' }
    if ($Startup) {
        New-Shortcut (Join-Path ([Environment]::GetFolderPath('Startup')) 'API Home.lnk') $tray
        Say '  starts with Windows'
    }
}

# keys are environment variables only; say which are missing, never ask for or store one
$keys = 'DEEPSEEK_API_KEY'
foreach ($k in $keys) {
    if (-not [Environment]::GetEnvironmentVariable($k, 'User') -and -not [Environment]::GetEnvironmentVariable($k, 'Machine') -and -not (Get-Item "env:$k" -ErrorAction SilentlyContinue)) {
        Say "  NOTE: $k is not set. Model calls need it: setx $k `"sk-...`"  (then restart API Home)"
    }
}

# 6. start
if (-not $NoLaunch) {
    Start-Process -FilePath $tray -WorkingDirectory $Dest
    Say '  started: look for the blue square in the taskbar (click the ^ arrow if hidden).'
}
Say ''
Say "  Done. Home page: http://127.0.0.1:8790/   Trouble? $Dest\TROUBLESHOOT.bat"
Say ''
