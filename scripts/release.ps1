# Release pipeline for the trial form (P1): build the signed installer,
# produce latest.json for the updater, and upload a GitHub Release.
# CI is intentionally paused; this script is THE release process.
#
# Prereqs (once):
#   - minisign keypair generated: npx tauri signer generate -w $env:USERPROFILE\.qiban\updater.key --password ""
#   - gh authenticated
# Usage:
#   powershell -File scripts/release.ps1            # full release of the current version
#   powershell -File scripts/release.ps1 -Draft     # create as draft for inspection
param([switch]$Draft)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
  $key = Join-Path $env:USERPROFILE '.qiban\updater.key'
  if (-not (Test-Path $key)) { throw "Signing key not found at $key. Generate it once: npx tauri signer generate -w $key --password `"`"" }

  $package = Get-Content (Join-Path $root 'package.json') | ConvertFrom-Json
  $version = $package.version
  $tag = "v$version"
  Write-Host "Releasing $tag"

  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $key -Raw
  try {
    # NOTE: the bundle-time signature step cannot prompt for this key's EMPTY
    # password unattended ($env:X = '' removes the var on Windows, so tauri
    # falls back to an interactive prompt that dies without a console). The
    # build still exits 0 without a .sig; we re-sign via the signer CLI below,
    # where the empty password rides as a real argv entry through cmd.
    npm run build:installer
  } finally {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
  }

  $setupDir = Join-Path $root 'target\release\bundle\nsis'
  $setup = Get-ChildItem $setupDir -Filter '*_x64-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if (-not $setup) { throw 'Installer not found after build.' }
  $sigFile = "$($setup.FullName).sig"
  if (-not (Test-Path $sigFile)) {
    Write-Host "Bundle unsigned after build; signing $($setup.Name) via signer CLI…"
    cmd /c "npx tauri signer sign -f `"%USERPROFILE%\.qiban\updater.key`" -p `"`" `"$($setup.FullName)`""
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $sigFile)) { throw "Signing failed for $sigFile" }
  }
  $signature = (Get-Content $sigFile -Raw).Trim()
  $sha256 = (Get-FileHash $setup.FullName -Algorithm SHA256).Hash

  # Updater manifest: the platform key matches tauri's Windows x64 target.
  $releaseAssetName = [uri]::EscapeDataString($setup.Name)
  $latest = [ordered]@{
    version = $version
    pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
      'windows-x86_64' = [ordered]@{
        signature = $signature
        url = "https://github.com/Thneoly/qiban-companion/releases/download/$tag/$releaseAssetName"
      }
    }
  }
  $latestPath = Join-Path $setupDir 'latest.json'
  $latest | ConvertTo-Json -Depth 5 | Set-Content $latestPath -Encoding utf8

  $notes = "栖伴 $tag`n`n安装包 SHA256：$sha256`n`n- 未签名安装包：首次运行请按验收单的 SmartScreen 引导操作。`n- 更新包经 minisign 验签后静默安装；老版本会自动提示升级。`n"
  $notesPath = Join-Path $setupDir 'RELEASE_NOTES.md'
  Set-Content $notesPath $notes -Encoding utf8

  $draftFlag = if ($Draft) { '--draft' } else { '' }
  Invoke-Expression "gh release create $tag `"$($setup.FullName)`" `"$latestPath`" `"$notesPath`" --title `"栖伴 $tag`" --notes-file `"$notesPath`" $draftFlag"
  Write-Host "Released $tag. SHA256 $sha256"
} finally {
  Pop-Location
}
