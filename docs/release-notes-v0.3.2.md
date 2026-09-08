这次更新修复视频时间、MiniMax 视频请求失败、对话输入焦点和中文显示，并改善多区域截图与教学录制。

### 视频理解与播放

- 修复 MiniMax 把视频事件时间压缩到零点几秒的问题。分析时使用校准后的副本，时间按原视频秒数显示；原录屏的画面、音频和文件保持不变。
- 修复部分非整数时长视频返回 HTTP 400 的问题：补齐分析副本的末尾帧，避免服务端拒绝不完整的帧组。
- 修复原位视频播放、暂停按钮不响应或图标与实际状态不同步的问题。
- 保留视频首轮回答与独立完整性核验；事件仍可在原位跳转、暂停和查看标记。模型给出的时间受采样精度影响，不保证逐帧准确。

### 对话输入、复制与中文

- 对话条每次弹出或从收纳状态恢复时，自动把输入焦点交给输入框；录制结束或暂停后可直接输入。
- 已显示时的回答刷新不会反复抢焦点，保留输入光标和选中的回答文字。
- 修复 Ctrl+C 无法复制 AI 回答的问题，复制内容保留中文、段落和表情。
- 中文回复使用与界面一致的微软雅黑字体，并默认要求简体中文；用户指定的语言、原文引用和代码保持原样。
- 首轮回答显示后可立即继续提问，后台补充标注会安全让出请求。

### 截图、标注与教学演示

- 新增跨截图关联箭头，可把一张截图中的对象与另一张截图中的对应内容连起来；单图导出保留该图的目标框与关联说明。
- 改善多个选区间工具条的悬停切换、边缘容错和延迟收起，修复对话条收纳后无法恢复的问题。
- 教学演示模式默认开启，并支持同时使用录屏和长截图；控制条自动避开采集区域。全屏没有空位时可按 F8 停止或完成，倒计时中按 F8 取消。
- 移除截图左上角的教学状态标识；可在设置中关闭教学演示模式。
- 改善逐行翻译的补译、空白行和输出格式处理，保留原始行序。

### 下载与升级

- 提供 Windows x64 安装 EXE、便携 ZIP 和 SHA256SUMS.txt 校验文件，安装包自带所需运行时。
- 可在设置中检查更新；使用 v0.2.5 及更早版本时，建议从发行页手动下载安装器升级。

[全部提交变化](https://github.com/abnste/mewu_ai/compare/v0.3.1...v0.3.2) · [完整版本记录](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)

---

This update fixes video timestamps, MiniMax video-request failures, conversation focus, copying, and Chinese text display. It also improves multiple selections and recording in teaching mode.

### Video analysis and playback

- Fixed MiniMax reporting events at fractions of a second instead of their original video times. Analysis uses a calibrated copy; the original recording, picture, audio, and file remain unchanged.
- Fixed HTTP 400 errors for some videos with fractional durations by completing the analysis copy's final frame group.
- Fixed in-place playback and pause controls failing to respond or showing the wrong state.
- Video answers retain an independent completeness check and local event navigation. Model timestamps remain limited by sampling precision and are not guaranteed to be frame-accurate.

### Conversation and text

- The input receives focus whenever the conversation bar appears or returns from its tucked-away state, including after recording finishes or preview playback is paused.
- Answer updates preserve the input caret and selected answer text while the bar is already visible.
- Fixed Ctrl+C copying of AI answers, preserving Chinese text, paragraphs, and emoji.
- Chinese replies use the same Microsoft YaHei font as the interface and default to Simplified Chinese. Explicit language requests, quotations, and code are preserved.
- Follow-up questions can be sent as soon as the first answer appears; background annotation repair yields safely.

### Capture, annotations, and teaching

- Added arrows connecting related objects across screenshots. Individual image exports retain the local target and relationship description.
- Improved toolbar switching across selections, pointer tolerance, delayed hiding, and restoration of the conversation bar.
- Teaching mode is on by default and supports recording and scrolling capture. Controls move outside the capture area; F8 stops or finishes capture when no controls fit, and cancels the recording countdown.
- Removed the teaching-status badge from the capture overlay. Teaching mode can still be disabled in Settings.
- Improved line-by-line translation recovery, blank-line handling, and response-format compatibility while preserving source-line order.

### Download and update

- Windows x64 installer EXE, portable ZIP, and SHA256SUMS.txt are provided. The required runtime is included.
- Check for updates in Settings. Users on v0.2.5 or earlier should download the installer manually from the release page.

[Full comparison](https://github.com/abnste/mewu_ai/compare/v0.3.1...v0.3.2) · [Release history](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)
