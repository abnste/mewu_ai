# 喵呜AI 0.5.9 / MewuAI 0.5.9

- 录制期间彻底移除全局鼠标钩子和鼠标事件重注入。遮罩与录制边框继续显示，覆盖层通过系统鼠标穿透把顶部暂停/停止控制条之外的输入直接交给底层应用，因此可正常点击、滚动并跨区域拖动窗口；拖动经过控制条也不会中断。
- 全屏录制没有安全控制条位置时保留全局 F8 停止入口；停止录制仍有有限超时恢复，不会无限停在“处理中”。
- 修复截图、标注、长截图和录制控制条的阴影重复描边与裁切；AI 标注文本卡的阴影改为带透明安全边距的独立图层，文字保持清晰。
- 框选预览复用冻结截图，降低拖动时的重复位图处理。

下载：[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Setup-0.5.9-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Portable-0.5.9-win-x64.zip)。

---

- Remove the global mouse hook and mouse-event re-injection entirely during recording. The dimmer and recording border remain visible while system mouse transparency sends input outside the top pause/stop strip directly to the underlying app, including window drags that cross the strip.
- Retain the global F8 stop action when a full-screen capture has no safe control-strip position. A bounded stop recovery prevents the UI from remaining on “Processing” indefinitely.
- Fix duplicated or clipped shadows on capture, annotation, long-capture, and recording controls. AI annotation text cards now use an independent shadow layer with transparent safety padding, keeping their text sharp.
- Reuse the frozen desktop image for selection previews to reduce repeated bitmap work while dragging.

Downloads: [installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Setup-0.5.9-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Portable-0.5.9-win-x64.zip).
