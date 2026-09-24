# 喵呜AI 0.5.8 / MewuAI 0.5.8

- 修复全屏录制倒计时结束后回退到普通截图工具条的问题；无安全空位的教学全屏采集隐藏控制条并保留 F8 完成/停止。
- 录制输入转发不再吞掉和重注入每个鼠标移动事件，避免区外点击拖垮全局输入；普通录制即使低级钩子不可用也不会被误取消。
- 录制窗口区域恢复增加无效 HWND 防护，避免录制结束时错误地关闭覆盖层。

下载：[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.8/MewuAI-Setup-0.5.8-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.8/MewuAI-Portable-0.5.8-win-x64.zip)。

---

- Fix full-screen recording falling back to the screenshot toolbar after the countdown; teaching captures with no safe control space hide the bar and retain F8 to finish or stop.
- Stop consuming and re-injecting every mouse move during recording so clicks outside the region cannot stall global input; ordinary recording no longer gets canceled when the low-level hook is unavailable.
- Guard recording-region restoration against an invalid HWND so the overlay is not closed spuriously after recording.

Downloads: [installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.8/MewuAI-Setup-0.5.8-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.8/MewuAI-Portable-0.5.8-win-x64.zip).
