# T12：语音口型

2026-10-07。分支 `feat/voice-mouth`（4 commits），接续[T11语音轮次](voice-chat-turn.md)。**朗读时嘴随真实音频振幅开合：`useVoiceTurn` 把播放元素接入 AnalyserNode，平滑后的振幅写入一个 ref，Live2D 每帧覆写 `ParamMouthOpenY`、SVG 档位嘴四档开合。** 这是 T12 的工程交付；真机视觉效果与参数校准属人工听感清单，不在此宣称。

## 链路与安全设计

- **分析**：`playClip` 把 `new Audio` 经 `MediaElementSource → AnalyserNode(fftSize 256) → destination` 接入共享 AudioContext；rAF 循环算 RMS，静音门限 0.02、线性放大 ×4 截到 1、150ms 指数平滑后写入 Pet 持有的 mouth ref。
- **声音优先于嘴**：只有 AudioContext 实际 `running` 才路由元素——挂起上下文会把元素输出吞掉（静音故障），宁可不动嘴。每次播放重试 resume；仍挂起则该句正常出声、嘴闭合。
- **生命周期**：每个 clip 的 `settle`（ended/error/停止）与 `interrupt`/`release` 都取消 rAF、断开 source、ref 归零；`startMouth` 的异步续体持 `settled` 令牌，迟到路由不会在 clip 结束后复活空闲循环；卸载时关闭 AudioContext。
- **React 共存**：SVG 档位嘴用 rAF 在渲染间隙直写 DOM 属性；JSX 属性值恒定，React 重渲染不会覆盖运动中的嘴。

## 两个渲染端

| 端 | 行为 | 静止时 |
|---|---|---|
| Live2D | ticker 在 `model.update` 后、render 前，振幅>0.02 每帧 `setParameterValueById('ParamMouthOpenY', min(1, 振幅))` 压制 motion 回写 | ≤0.02 不写，motion 立即夺回；`prefers-reduced-motion` 分支永不写；无此参数的模型安全无效果 |
| SVG | 振幅分 4 档开合（ry 1.5/3/4.5/6），椭圆随档显示 | 归零隐藏椭圆、恢复原状态嘴线 |

## 验证证据

- `voice-chat.spec` 15 例全绿（原 9 例回归 + 新档位用例）：测试内强制 AudioContext 门为 running、按需合成 Analyser 方波数据——响输入升到第 4 档、静音经 150ms 衰减回 0 档、原嘴线恢复；断言走的是 analyser→平滑→ref→SVG 全链。
- `live2d`/`pet` 套件绿（新 prop 不破坏挂载与降级路径）；全套门禁（cargo fmt/clippy/test、`npm run check`、Playwright 全套、`verify:docs`）通过。
- Live2D 参数覆写的视觉效果依赖授权模型样本（`QIBAN_TEST_LIVE2D=1` 门控用例）与真机，未在本轮验证。

## 边界

1. **真机未跑**：WebView2 的 autoplay 策略下 AudioContext resume、真实 Analyser 振幅、Live2D 视觉效果均待 native 脚本与人工清单（与 T11 验收同批：需验收 exe 重建）。
2. 参数是首版估值（门限 0.02、放大 ×4、平滑 150ms、4 档分界 0.25/0.5/0.75），合成数据验证过链路但未按真实语音校准；听感清单复核后调整。
3. 挂起上下文时嘴不动（设计取舍：声音优先）；该状态真机是否出现、如何呈现待清单记录。
4. reduced-motion 用户无口型动画（沿用既有 reduced-motion 立场）。
5. 口型只跟随已播放音频：合成等待期与提示音段嘴不动（提示音振幅会驱动开合，属预期）。
6. 口型不进历史、不落盘、不影响聊天链——表现层，与 T11「历史只存文本」一致。

## 人工听感清单增项（真机同批复核）

口型与语音的同步感（rAF+150ms 平滑是否迟滞）/ 句间与提示音段的开合是否自然 / 档位跨度是否够（4 档还是需要连续）/ 打断与失焦是否立即闭嘴 / Live2D 模型口型幅度是否被 motion 干扰。

接续：验收 exe 重建后运行 T11 native 脚本与人工清单（含本文件增项）；backlog 中 T12 标注为「工程交付，真机验收待补」。
