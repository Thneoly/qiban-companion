$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Push-Location $workspace
$oldCargoHome = $env:CARGO_HOME
$oldTarget = $env:CARGO_TARGET_DIR
try {
    if (!$env:CARGO_HOME -and (Test-Path (Join-Path $workspace '.cache/cargo'))) { $env:CARGO_HOME = Join-Path $workspace '.cache/cargo' }
    $env:CARGO_TARGET_DIR = Join-Path $workspace 'target'
    & npm.cmd run build:mobile
    if ($LASTEXITCODE -ne 0) { throw 'Mobile build failed.' }
    & cargo test -p companion-coordinator --locked --lib mobile_web_browser_integration -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Mobile integration failed.' }
} finally {
    $env:CARGO_HOME = $oldCargoHome
    $env:CARGO_TARGET_DIR = $oldTarget
    Pop-Location
}
