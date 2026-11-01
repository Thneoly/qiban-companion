# verify-probes.ps1 - LOCAL pixel verification of smoke probe frames (never uploaded).
# Prints region stats; thresholds judged by the operator.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function RegionStats($path, $name, $x0, $y0, $x1, $y1) {
  if (-not (Test-Path $path) ) { Write-Output "$name : FILE_MISSING"; return }
  $b = New-Object System.Drawing.Bitmap($path)
  $r = 0; $g = 0; $bl = 0; $n = 0; $bright = 0; $dark = 0
  for ($x = $x0; $x -lt $x1; $x += 6) {
    for ($y = $y0; $y -lt $y1; $y += 6) {
      if ($x -ge $b.Width -or $y -ge $b.Height) { continue }
      $c = $b.GetPixel($x, $y)
      $r += $c.R; $g += $c.G; $bl += $c.B; $n++
      $lum = 0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B
      if ($lum -gt 190) { $bright++ } elseif ($lum -lt 90) { $dark++ }
    }
  }
  $b.Dispose()
  Write-Output ("{0} : avgRGB=({1},{2},{3}) bright%={4} dark%={5}" -f $name, [int]($r/$n), [int]($g/$n), [int]($bl/$n), [int](100*$bright/$n), [int](100*$dark/$n))
}

$out = Join-Path $here 'out'
# wallpaper reference region (empty area, left-top) for every frame
foreach ($f in @('probe-p0-baseline','probe-p2b-fullbody','probe-p3-wide','probe-p4-compact','probe-p6-gdi','probe-p6-dda-frame','probe-p6-dda-live','probe-p2b-panel')) {
  $p = Join-Path $out "$f.png"
  RegionStats $p "${f}[wall-ref 150..600,150..450]" 150 150 600 450
  RegionStats $p "${f}[pet-window 1470..1750,610..1020]" 1470 610 1750 1020
  RegionStats $p "${f}[character 1556..1736,744..1004]" 1556 744 1736 1004
  RegionStats $p "${f}[wide-win 1030..1750,330..1020]" 1030 330 1750 1020
  RegionStats $p "${f}[phone 570..966,130..990]" 570 130 966 990
  Write-Output ''
}
