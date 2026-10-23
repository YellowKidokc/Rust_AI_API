# Stops API Home, removes its programs and shortcuts. Config, _data (ledger) and LOGS stay unless -All.
param(
    [string]$Dir = '',
    [switch]$All
)
$ErrorActionPreference = 'Continue'
if (-not $Dir) { $Dir = Split-Path -Parent $MyInvocation.MyCommand.Path }
Get-Process forge-tray, forge -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($Dir, [StringComparison]::OrdinalIgnoreCase) } |
    ForEach-Object { Write-Host "stopping $($_.Name) ($($_.Id))"; Stop-Process -Id $_.Id -Force }
Start-Sleep -Milliseconds 500
foreach ($folder in 'Programs', 'Desktop', 'Startup') {
    $f = [Environment]::GetFolderPath($folder)
    if (-not $f) { continue }
    $l = Join-Path $f 'API Home.lnk'
    if (Test-Path $l) { Remove-Item $l -Force; Write-Host "removed $l" }
}
if ($All) {
    Write-Host "removing $Dir"
    Set-Location $env:TEMP
    Remove-Item $Dir -Recurse -Force
} else {
    foreach ($f in 'forge.exe', 'forge-tray.exe', 'LAUNCH.bat') {
        $p = Join-Path $Dir $f
        if (Test-Path $p) { Remove-Item $p -Force; Write-Host "removed $p" }
    }
    Write-Host "kept: $Dir (config, _data, LOGS). UNINSTALL.bat -All removes it."
}
