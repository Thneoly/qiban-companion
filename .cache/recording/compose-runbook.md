# TUWE-5 v2 录制会话 runbook（背景返工：壁纸 + 无任务栏采集）

2026-11-01 返工定的口径（PM 派单）：任务栏 = 合成统一裁切 + 采集区域从源头排除；
背景 = 方案 A 干净系统风壁纸（程序生成 `wallpaper.png`，零版权）。重录镜 1/2/3/5
（镜 4/7 视构图——本方案下 4/7/6 也全部来自新 take，见下），镜 2 并入同一长镜。

## 实测几何（2026-11-01 冒烟验证）

- 屏幕 1920x1080 @100% DPI；任务栏 rect T=1032，高 48px。
- 采集：`ddagrab=output_idx=0:framerate=30:video_size=1920x1032:offset_x=0:offset_y=0`
  （1920x1032，任务栏永不入镜；宠物窗 320x440@1450,592，底缘恰在 1032）。
- 合成统一链：`crop=1920:1032:0:0`（对区域素材是 no-op，纯保险）→ `scale=1920:1080`
  （回放缩 1080p，纵向 4.65%，视觉不可辨；横向零损失，宠物不裁边）。
- y 坐标映射：out_y = src_y × 1080/1032（×1.04651）。旧对话区模糊 src y352-552 → out
  crop=740:209:1018:368；边界行 src y726-758 → out crop=740:34:1018:760。
  这两个矩形开拍后仍要从实际帧复测微调（气泡实际落点为准）。
- 镜 7 特写源：宠物可视区 ≈ x1490-1750, y790-1032（白兔+底座+浅字），取
  crop=300:272:1470:760，模糊填充扩 1080p（壁纸底模糊后天然协调）。

## 会话步骤（开录当天，招呼 Thne 离开键鼠后）

1. 起依赖：`cd .cache/recording && npm i`（ws/playwright；package.json 在，node_modules
   不入库）。`CARGO_TARGET_DIR=D:\Game\target` 构建
   `npm run tauri --workspace @companion/desktop -- build --no-bundle --config .cache/rec-video.conf.json`
   （apps/desktop 下的覆盖配置，补 updater 假端点；缓存命中约 40s 内）。
2. 起 OBS（若未运行）→ `node obs-ctl.mjs setup` → `node obs-ctl.mjs status` 确认。
3. 起 app：`$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'`
   后启动 `D:\Game\target\release\companion-desktop.exe`，等 CDP `/json` 出现 pet/panel 页。
4. 壁纸背景窗（顺序关键——bg 先 topmost，再把宠物压回它上面）：
   `Start-Process powershell -NoProfile -File bgview.ps1 -Image wallpaper.png -Topmost`
   然后 `powershell -NoProfile -File petwin.ps1 -Action top -Which pet`。
   bgview 枚举赋值必须走字符串（'Stretch'）——类型字面量解析会静默炸掉背景窗，
   这就是 v1 草片「部分镜头背景窗没盖住任务栏」与白屏闪帧丢失的最可能根因。
5. `node take-driver.mjs`（内部顺序：隐藏杂散浮窗 → OBS start → ddagrab 区域采集 →
   白闪+音调同步标记 → 镜1 待机 → 镜2 拖拽（拖完 petwin 挪回原位）→ 镜3 打字 →
   镜5 待办 → 镜4 语音（含 12s settle 重试）→ 镜6 配对（B 视口）→ 镜7 特写素材尾 →
   停止 → 还原杂散浮窗）。产物 `out/take.mkv`（约 4-5 分钟）+ `obs-out/`。
6. 从 take 的白闪/音调时刻对齐 OBS 音轨偏移（旧法：问题音与 TTS 结束双重对齐，
   v1 常数是 +4.93s，v2 重测）。抽帧核对：无任务栏、无桌面、无浮窗、宠物/气泡落点。
7. 把实际时间戳填进 compose-avfilter.txt 的 S2A..S7B / A4A..A4D / TOTAL 占位符，
   然后：
   `ffmpeg -y -i out/take.mkv -i <obs-mkv> -filter_complex_script compose-avfilter.txt -map [vout] -map [aout] -c:v libx264 -crf 19 -pix_fmt yuv420p -c:a aac draft-v2.mp4`
8. 逐帧红线自查（key/设置页/接口地址/真实数据/内部工具/任务栏），OBS 原始音轨
   （含环境人声）用完即删。回帖交验收。

## 红线与既有结论（不变）

- key/设置页/接口地址不入境；「发往模型服务」边界行与旧对话区模糊遮挡；
  全程测试数据；不用「开箱即用」话术；每步只报实际执行结果。
- Thne 常在机器前：开录前回帖招呼、短拍快收；真实模型调用已授权（成本口径不变）。
- 镜 4 语音：点麦后等 3s 再播问题音（q4_loud.wav）；轮后异步记忆对账会取消下一轮，
  间隔 ≥12s 重试。中文 drawtext 走 -filter_complex_script 文件（本文件目录即 UTF-8 无 BOM）。
- PixPin 贴图等浮窗由 stray-windows.ps1 自动收放；它只动「真的渲染进 DDA 的窗口」
  （DWM cloaked 的 UWP 幽灵窗不动——v1 版脚本曾误挪设置窗，已修复并实测还原精确）。
