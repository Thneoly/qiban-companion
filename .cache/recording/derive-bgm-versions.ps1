# derive-bgm-versions.ps1 - BGM increment for draft-v2 when the take material is
# gone: rebuild the deterministic BGM stem, subtract it from the delivered file
# to recover the voice stem, then emit the no-music cut and a ducked-BGM cut.
# Empirical constants measured 2026-11-01 against draft-v2.mp4 (runbook v2.3):
#   stem alignment  = 239 samples @48k (~4.98 ms) late vs the compose graph
#   effective level = volume 0.16782 after aformat mono->stereo (1/sqrt2 upmix;
#   the session had hand-tuned the bed from the committed 0.16 to ~0.168)
# Voice windows for reference: a4a 30.7-33.7s, a4b 36.2-43.2s, total 58.7s.
# Usage: run in .cache/recording with draft-v2.mp4 present (issue attachment).
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if (-not (Test-Path bgm.wav)) { & powershell -NoProfile -File make-bgm.ps1 }

# 1) decode delivered audio
& ffmpeg -y -hide_banner -loglevel error -i draft-v2.mp4 -vn -c:a pcm_f32le draft-audio.wav

# 2) aligned BGM stem, per-channel level equal to the shipped mix
& ffmpeg -y -hide_banner -loglevel error -i bgm.wav -af "atrim=0:58.7,asetpts=PTS-STARTPTS,aformat=channel_layouts=stereo,volume=0.16782,afade=t=in:st=0:d=1.5,afade=t=out:st=57.2:d=1.5,adelay=239S:all=1" -c:a pcm_f32le bgmstem.wav

# 3) voice stem = delivered mix minus stem (sample-exact; residual -77 dB RMS)
& ffmpeg -y -hide_banner -loglevel error -i draft-audio.wav -i bgmstem.wav -filter_complex "[0:a][1:a]amerge=inputs=2,pan=stereo|c0=c0-c2|c1=c1-c3,atrim=0:58.7" -c:a pcm_f32le amain-rec.wav

# 4) no-music cut (video stream bit-identical to the delivered file)
& ffmpeg -y -hide_banner -loglevel error -i draft-v2.mp4 -i amain-rec.wav -map 0:v -map 1:a -c:v copy -c:a aac -b:a 128k draft-v2-nomusic.mp4

# 5) ducked-BGM cut: voice is the sidechain; bed drops ~16.6 dB under speech.
#    NB: asplit is REQUIRED - feeding one label to both the sidechain input and
#    amix silently produces a wrong mix (measured +6.5 dB on the bed).
& ffmpeg -y -hide_banner -loglevel error -i draft-v2.mp4 -i amain-rec.wav -i bgmstem.wav -filter_complex "[1:a]asplit=2[v1][v2];[2:a][v1]sidechaincompress=threshold=0.008:ratio=4:attack=120:release=800:detection=rms[bgd];[v2][bgd]amix=inputs=2:duration=first:dropout_transition=0:normalize=0,alimiter=limit=0.95[aout]" -map 0:v -map "[aout]" -c:v copy -c:a aac -b:a 128k draft-v2-bgm-ducked.mp4

# 6) verify: the three video-stream md5s must be identical
& ffmpeg -hide_banner -loglevel error -i draft-v2.mp4 -map 0:v -c copy -f md5 -
& ffmpeg -hide_banner -loglevel error -i draft-v2-nomusic.mp4 -map 0:v -c copy -f md5 -
& ffmpeg -hide_banner -loglevel error -i draft-v2-bgm-ducked.mp4 -map 0:v -c copy -f md5 -
Write-Output 'done: draft-v2-nomusic.mp4 + draft-v2-bgm-ducked.mp4'
