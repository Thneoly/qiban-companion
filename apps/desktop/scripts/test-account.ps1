$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$oldCargoHome = $env:CARGO_HOME
$oldTarget = $env:CARGO_TARGET_DIR
Push-Location $workspace
try {
    if (!$env:CARGO_HOME -and (Test-Path (Join-Path $workspace '.cache/cargo'))) { $env:CARGO_HOME = Join-Path $workspace '.cache/cargo' }
    $env:CARGO_TARGET_DIR = Join-Path $workspace 'target'
    & npm.cmd run build:mobile
    if ($LASTEXITCODE -ne 0) { throw 'Mobile build failed.' }
    & cargo test -p companion-desktop --locked --lib account::tests -- --include-ignored
    if ($LASTEXITCODE -ne 0) { throw 'Native account integration failed.' }
} finally {
    $env:CARGO_HOME = $oldCargoHome
    $env:CARGO_TARGET_DIR = $oldTarget
    Pop-Location
}
