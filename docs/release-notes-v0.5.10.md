# 喵呜AI 0.5.10 / MewuAI 0.5.10

这是一个合并发行版，汇总 0.5.3 至 0.5.9 的公开更新，并包含本次录屏声音稳定性修复。这样删除旧发行版后，仍可在一个版本中查到完整的这段更新记录。/ This consolidated release includes the public changes from 0.5.3 through 0.5.9 plus the recording-audio stability fix, so the complete update history for this range remains available after older releases are removed.

## 更新内容 / Changes

- **0.5.3–0.5.5：框选与编码稳定性。** 框选拖动复用冻结桌面图像，减少重复裁剪造成的卡顿；录屏使用更稳定的 H.264 编码路径，并加入 100% 与 175% DPI 的像素一致性回归。/ **0.5.3–0.5.5: selection and encoding stability.** Reuse the frozen desktop image during selection dragging, use a more stable H.264 recording path, and add pixel-consistency coverage at 100% and 175% DPI.
- **0.5.6：历史会话。** 新建对话会清空当前聊天列表并保持独立上下文；历史按钮可以恢复之前的完整会话，当前进程内的历史也能正常列出。/ **0.5.6: conversation history.** New conversations clear the current chat list and keep an isolated context; the history button restores complete earlier sessions, including sessions from the current process.
- **0.5.7：录制输入与阴影。** 录制时控制条外的鼠标输入交给底层应用，控制条继续支持暂停、继续和停止；录制、截图和标注工具条改用独立阴影图层。/ **0.5.7: recording input and shadows.** Mouse input outside the recording bar goes to the underlying application while pause, resume and stop remain available; recording, capture and annotation bars use independent shadow layers.
- **0.5.8：全屏录制启动。** 修复倒计时结束后回到普通截图工具条的问题；全屏没有安全控件位置时保留 F8 停止；移除会吞掉并重注入鼠标移动的路径，增加窗口区域恢复保护。/ **0.5.8: full-screen recording startup.** Fix the fallback to the screenshot toolbar after countdown; retain F8 stop when no safe control position exists; remove mouse-movement consumption and reinjection, and guard window-region restoration.
- **0.5.9：录制交互重构。** 完全移除录制期间的全局鼠标钩子和事件重注入；遮罩与录制边框持续显示，区外点击、滚动和跨区域拖动交给底层应用；停止流程有限等待，避免永久停在“处理中”；修复控制条、截图、长截图和 AI 标注卡的阴影裁切与重复描边。/ **0.5.9: recording interaction redesign.** Remove global mouse hooks and event reinjection during recording; keep the dimmer and recording border visible while clicks, scrolling and cross-region drags outside the bar reach the underlying application; bound stop recovery so processing cannot hang forever; fix clipped and duplicated shadows on controls, scrolling captures and AI annotation cards.
- **0.5.10：电脑声音录制稳定性。** 修复静音桌面开启电脑声音录制时约 3 秒后视频画面停留在旧帧的问题。录制使用选定输出设备的共享 WASAPI 时钟保持音频链路连续，同时保留真实声音、麦克风开关和原有音量设置；设备异常会有界停止并给出可操作提示。/ **0.5.10: computer-audio recording stability.** Fix video remaining on an old frame after roughly three seconds when computer-audio recording is enabled on a silent desktop. A shared WASAPI clock on the selected output keeps the audio path active while preserving real sound, microphone settings and existing volume behavior; device failures stop within a bound with an actionable message.

## 验证 / Validation

- 普通与教学模式的 2560×1440 全屏动态录制持续 12.5 秒，实际预览和独立解码帧均持续更新；覆盖暂停恢复以及静音 → 有声 → 静音切换。/ 2560×1440 full-screen dynamic recordings in normal and teaching modes ran for 12.5 seconds with continuous updates in both the in-place preview and independent decoding; coverage includes pause/resume and silence → sound → silence transitions.
- Release 构建 0 警告、0 错误；全量测试 1549 项通过、1 项既有现场音频测试默认跳过；启用 `MEWU_AUDIO_LIVE=1` 后声音相关 10 项全部通过。/ Release builds completed with 0 warnings and 0 errors; all 1549 tests passed with 1 pre-existing interactive-audio test skipped by default; with `MEWU_AUDIO_LIVE=1`, all 10 audio tests passed.

## 下载 / Downloads

[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.10/MewuAI-Setup-0.5.10-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.10/MewuAI-Portable-0.5.10-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看，不附带单独校验文件。/ [Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.10/MewuAI-Setup-0.5.10-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.10/MewuAI-Portable-0.5.10-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in GitHub asset details; no separate checksum file is included.

完整历史仍保留在 [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.5.10/CHANGELOG.md)。/ The full history remains in [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.5.10/CHANGELOG.md).
