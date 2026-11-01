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
   白闪+音调同步标记 → 镜1 待机 → 镜2 拖拽（拖完 petwin 挪回原位）→ **镜2B 形象切换
   （2026-11-01 增派，详见下节）** → 镜3 打字 → 镜5 待办 → 镜4 语音（含 12s settle
   重试）→ 镜6 配对（B 视口）→ 镜7 特写素材尾 → 停止 → 还原杂散浮窗）。
   产物 `out/take.mkv`（约 5 分钟）+ `obs-out/`。
6. 从 take 的白闪/音调时刻对齐 OBS 音轨偏移（旧法：问题音与 TTS 结束双重对齐，
   v1 常数是 +4.93s，v2 重测）。抽帧核对：无任务栏、无桌面、无浮窗、宠物/气泡落点。
7. 把实际时间戳填进 compose-avfilter.txt 的 S2A..S7B / **S2BA..S2BF（镜2B 三拍）** /
   A4A..A4D / TOTAL 占位符，然后：
   `ffmpeg -y -i out/take.mkv -i <obs-mkv> -filter_complex_script compose-avfilter.txt -map [vout] -map [aout] -c:v libx264 -crf 19 -pix_fmt yuv420p -c:a aac draft-v2.mp4`
8. 逐帧红线自查（key/设置页/接口地址/真实数据/内部工具/任务栏），OBS 原始音轨
   （含环境人声）用完即删。回帖交验收。

## 镜 2B 形象切换（2026-11-01 全链路真机冒烟通过）

PM 12:04 增派：展示「换成你自己的 Live2D 模型」，插镜 2 与镜 3 之间，全片超时先裁这镜
和镜 5。口径红线：字幕只说「换成你自己的 Live2D 模型」，不出现「多形象/形象库」暗示。

**素材许可（已核对，出镜放行的依据）**：本机 Hiyori 来自 CubismWebSamples 固定提交
（`licenses/samples-LICENSE.md` 随素材落地），属 Live2D Free Material License + 逐模型
Sample Terms。Thne 为个人（General User），公开视频（含商用）在许可内；**硬性义务是标注
版权声明**——短版 "This content uses sample data owned and copyrighted by Live2D Inc."
已加进 compose-avfilter.txt 的镜 7 片尾卡（y=1010 小字），群发文案尾注同款由 PM/Thne 侧
跟进。禁止项均不触碰（不改设计、不再分发模型文件、不用素材附带音声）。

**实拍流程（take-driver 已内置，全部真机验证过）**：气泡 → 更多 → 角色与场景（CDP）→
真实点击「导入模型文件夹」（label 中心 = 宠物窗原点 + (152,211)，运行时重测）→ 原生
文件夹选择器弹出（标题「选择要上传的文件夹」，约 960×540@(960,456)）→ **立即
`dialog-probe.ps1 -Topmost` 置顶**（实测 topmost 壁纸会整个盖住选择器，不置顶镜头里
点击落空）→ input.ps1 text `D:\Live2D\Hiyori` + Enter×2（type-ahead 导航，实测可用）→
选择器关闭后 webview 内弹出**权限气泡「上传到此站点？」**（Chromium 权限 UI，非 Win32
窗体，dialog-probe 看不到；停靠宠物窗左上，「上传」按钮 ≈ 窗原点 + (212,101)）→ 点击
「上传」→ 正在保存 → Hiyori 上屏待机（canvas 180×260，比栖栖小；两帧差分确认在动）。
拍完镜头内 CDP 清场：移除本机模型 → 返回互动 → 收起气泡，镜 3 原流程不受影响。

**成片剪辑（compose-avfilter.txt 已加段）**：三拍硬切——面板打开 ~1.2s（S2BA/B）、
点击+选择器+路径 ~2.4s（S2BC/D）、Hiyori 待机 + 字幕 ~2.4s（S2BE/F）；权限气泡与
等待期剪掉（真实发生、只做剪辑省略，不冒充）。a2b 静音轨 6.0s，concat n=9。

**兜底（不必用则不用）**：若会话中键盘导航失灵——Esc 取消选择器后走 Playwright 注入：
`chromium.connectOverCDP('http://127.0.0.1:9222')` → pet 页
`locator('input[aria-label="导入模型文件夹"]').setInputFiles('D:/Live2D/Hiyori')`
（webkitdirectory 输入**必须传目录路径**，传文件列表会报错；目录注入保留
webkitRelativePath，importModel 校验通过——`avatar-smoke.mjs inject` 即此路径，已验证）。
镜头口径退为「点击导入 → 切换瞬间」，如实。

**冒烟时序参考（2026-11-01 实测）**：点击→选择器弹出 ~2s；text 18 字符 ~2s；Enter 后
导航 ~2s；权限气泡在选择器关闭后 ~1.5s 内出现；点「上传」→ 2D 数字人 选中 ~2-4s
（18 文件 4.7MB 入 IndexedDB）。窗口几何：宠物窗 320×440@1450,592（home）。

## 踩坑补记（2026-11-01 冒烟实测）

- ddagrab 短间隔内多次单帧获取会挂起（wedge，ffmpeg 停在首帧前不动）——已知现象，
  放置自愈；单帧探针可用 gdigrab 兜底（gdigrab 单帧能拍到宠物窗，只是连录不行），
  取回 1920x1080 后自己 crop 顶部 1032 即可。
- drawtext / 含中文与引号的滤镜一律走 -filter_complex_script 文件
  （preview-look-filter.txt 是样例）——内嵌进 PS 命令行的引号会被拆坏整条命令，
  甚至把后半段参数误当路径执行（实测两次）。
- 冒烟顺序纪律：起 bg 后必须确认 bgview 进程活着、再开始采集——bgview 炸掉时
  采集拍到的是用户真实桌面（2026-11-01 首次冒烟即如此，帧文件本地即删、未外发）。

## v2.2 执行补记（2026-11-01 下午返工会话，take B + 镜 6 补拍 + BGM）

- **成片**：`compose-avfilter-v2.txt`（2B 两拍：面板+光标 → Hiyori+字幕；S6 取自补拍件
  `[2:v]`；BGM `amix ... normalize=0`——默认 normalize 会把人声压半；全 concat 段补
  `setsar=1`，gdigrab 件 SAR=675:1088 不齐会炸 concat）。BGM 用 `make-bgm.ps1` 合成。
- **原生文件夹选择器路线当日失效**：对话框「上传」点掉后 input.files 仍为 0、权限气泡
  不再出现；且任何 picker 交互会楔死该 WebView2 实例后续的 CDP `setFileInputFiles`
  （重启 app 才恢复）。会话内绝不碰原生 picker，导入一律 `inject-2b.mjs`（CDP 目录注入，
  镜头口径「点击导入 → 切换瞬间」），take-driver 已按此改。
- **镜 6 配对码**：panel 页停在「日常」时按钮在隐藏 DOM 里被盲点、码不显示——必须先点
  `a.nav-link`「设备」再取码（take-driver 已加导航 + 重试 + 失败抛错）。
- **ddagrab 会话级楔死**：一场连录结束后短间隔再起 ddagrab 会停在首帧前（不建输出文件）。
  补拍小段改 gdigrab 连录（普通窗口 OK；宠物分层窗不渲染，宠物缺席段构图书面规避）。
  起录后先验证输出文件在增长再放行动作（`phone-shot6.mjs` 的 p6-* 文件信号即为此设计）。
- **OBS 启动**：`Start-Process` 必须带 `-WorkingDirectory`（bin\64bit），否则报
  `Failed to find locale/en-US.ini`；崩溃残留会弹「检测到崩溃」对话框，UIA 点
  「以正常模式运行」（安全模式禁用 websocket）。
- **playwright 钉 1.64.1**：`^1.49` 漂到要 chromium-1248（未装，手机浏览器起不来）；
  1.64 + `executablePath` 指向已装的 chromium-1243 解决（connectOverCDP 本就不需要本地浏览器）。
- **PS 5.1 读写 UTF-8 滤镜文件会毁中文**（本次重蹈：Get-Content/Set-Content 往返后
  drawtext 全部乱码）——含中文的脚本/滤镜文件只许用 Write/Edit 工具改。

## 红线与既有结论（不变）

- key/设置页/接口地址不入境；「发往模型服务」边界行与旧对话区模糊遮挡；
  全程测试数据；不用「开箱即用」话术；每步只报实际执行结果。
- Thne 常在机器前：开录前回帖招呼、短拍快收；真实模型调用已授权（成本口径不变）。
- 镜 4 语音：点麦后等 3s 再播问题音（q4_loud.wav）；轮后异步记忆对账会取消下一轮，
  间隔 ≥12s 重试。中文 drawtext 走 -filter_complex_script 文件（本文件目录即 UTF-8 无 BOM）。
- PixPin 贴图等浮窗由 stray-windows.ps1 自动收放；它只动「真的渲染进 DDA 的窗口」
  （DWM cloaked 的 UWP 幽灵窗不动——v1 版脚本曾误挪设置窗，已修复并实测还原精确）。

## v2.3 BGM 增量（2026-11-01 下午，接 PM 12:50 配乐派单：零版权 + 镜 4 闪避 + 两版交付）

- **背景**：v2.2 会话 13:46 交付 draft-v2.mp4（恒定音量配乐、**无闪避**）后中断，take
  与中间产物随 worktree 清理丢失（平台收尾清理故障第 6 次）。补救路线 = BGM 生成链全
  确定性（纯正弦 aevalsrc），从成片反推：`make-bgm.ps1` 重建 BGM → `align-bgm.mjs`
  互相关对齐（实测滞后 239 样本 ≈4.98ms；成片实际音量 = 上混后 0.168，即上会话把滤镜
  里的 0.16 手调过）→ 从成片音频逐样本减出人声轨（静音段残差 -77dB RMS，采样级干净）。
- **两版输出**（`derive-bgm-versions.ps1` 一键复现；视频流与 draft-v2 逐位相同，音频多
  一代 128k AAC、人声段电平实测与原版一致）：
  - `draft-v2-nomusic.mp4`：无音乐版，除镜 4 人声外为数字静音；
  - `draft-v2-bgm-ducked.mp4`：配乐版，sidechaincompress 人声侧链闪避——对话段 BGM
    压低 16.6dB（人声 -20dB 时配乐 -46dB，低 26dB），非对话段电平与已交付版一致
    （-29.3 vs -29.7dB，听感不变）；句间停顿 800ms 内自然回升，属闪避正常行为。
- **坑三条（当日实测）**：① `adelay=239` 的单位是毫秒不是样本——样本要写 `239S`
  后缀，差 48 倍且不报错；② 同一标签同时喂 sidechaincompress 侧链与 amix 会**静默**
  出错误混音（BGM 床 +6.5dB），必须 `asplit` 显式分叉；③ `aformat` 的 mono→stereo
  上混带 1/√2 增益，对齐成片音量时要算进去。
