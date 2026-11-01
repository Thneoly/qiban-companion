# make-bgm.ps1 - synthesize the zero-copyright ambient BGM bed (4 soft chords x2, 60s).
# Thne requested light background music 2026-11-01; no third-party material, same policy
# as the wallpaper. Output: bgm.wav (48k s16 stereo, lowpass + slow tremolo applied).
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$pairs = @(@(220.00,261.63,329.63),@(174.61,220.00,261.63),@(130.81,164.81,196.00),@(196.00,246.94,293.66))
$seq = @(0,1,2,3,0,1,2,3)
$i = 0
foreach ($k in $seq) {
  $f = $pairs[$k]
  $expr = '0.3*sin(2*PI*' + $f[0] + '*t)+0.3*sin(2*PI*' + $f[1] + '*t)+0.3*sin(2*PI*' + $f[2] + '*t)'
  & ffmpeg -y -f lavfi -i "aevalsrc=exprs=$expr`:s=48000:d=7.5" -af "afade=t=in:st=0:d=0.8,afade=t=out:st=6.7:d=0.8" -c:a pcm_s16le "bgm-$i.wav" -hide_banner -loglevel error
  $i++
}
(0..7 | ForEach-Object { "file 'bgm-$_.wav'" }) | Out-File -Encoding ascii bgmlist.txt
& ffmpeg -y -f concat -safe 0 -i bgmlist.txt -af "lowpass=f=1100,tremolo=f=0.13:d=0.35" -c:a pcm_s16le bgm.wav -hide_banner -loglevel error
Remove-Item bgm-*.wav, bgmlist.txt -Force
Write-Output 'bgm.wav ready (60.0s)'
