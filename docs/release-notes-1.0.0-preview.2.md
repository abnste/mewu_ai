# 喵呜AI / MewuAI 1.0.0-preview.2

## 简体中文

Windows x64 重制版预览版 2。

- 完善截图与电子黑板：选区、图片、录屏和 HTML/SVG 结果可带入黑板，TXT 笔记可直接编辑；对象通过顶部悬浮横条移动、拖入「删除」移除，图片支持滚轮缩放，橡皮只擦笔迹。
- 统一工具条按钮和 RGB/HEX 色板，保留最近三种颜色；画笔大小改为滑条，右键橡皮与画笔同大小，修复点状笔迹擦除和绘制闪烁。
- 恢复原版随长度变化的线条箭头；荧光笔按住 Shift 可画水平、垂直和 45° 斜线。文字直接在画布编辑，序号文字随底色切换明暗。
- 完善会话最小化、Esc 与历史恢复：悬浮列表只显示主动最小化的会话，实时显示思考和回答，完成后显示最终答复开头。添加引用在输入框光标处插入 @图片或 @视频；首次回复完成后可自动生成标题，可在设置中关闭。
- 修复原位标注格式兼容和译文图层读取；交互式 HTML/SVG/Canvas 结果保持透明，并提供悬浮移动、刷新与关闭控件。
- 长截图增加实时侧边预览，视频操作与快速裁切默认置于视频下方。数据页显示历史文件、截图、会话占用，并提供受引用保护的清理操作。

预览版 1 可从「设置 → 关于」检查更新。旧版 API 连接支持一次导入；旧版会话历史暂不迁移。0.7.x 正式版不会自动切换到预览版。

## English

Windows x64 remake preview 2.

- Improve capture and the whiteboard: capture regions, images, recordings and HTML/SVG results can become board objects; TXT notes can be edited directly. Move objects using the top hover bar, resize images with the wheel and remove objects by dropping them onto Delete. The eraser only removes ink.
- Unify square toolbar buttons and the RGB/HEX palette with three recent colors. Use a brush-size slider and a matching right-button eraser; fix single-point erasing and drawing flicker.
- Restore length-adaptive open arrows. Hold Shift with the highlighter for horizontal, vertical or 45° strokes. Edit text directly on the canvas; numbered labels adapt their text contrast to the fill.
- Improve minimized conversations, Escape and history restore. Only explicitly minimized chats enter the floating list, with live reasoning and answer previews and the final answer opening after completion. References insert @image or @video at the input caret. Optional automatic titles run after the first completed reply.
- Fix in-place annotation format compatibility and translation-layer loading. Interactive HTML/SVG/Canvas results remain transparent with hover controls for moving, reloading and closing.
- Add live side previews for long captures. Place video actions and quick trim below the video by default. Show file, image and conversation storage usage with cleanup that protects referenced content.

Preview 1 users can check for updates in Settings → About. Legacy API connections can be imported once; legacy conversation history is not migrated yet. Stable 0.7.x installations do not automatically switch to previews.

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v1.0.0-preview.2/MewuAI-Remake-Setup-1.0.0-preview.2-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v1.0.0-preview.2/MewuAI-Remake-Portable-1.0.0-preview.2-win-x64.zip)

[对应版本源码 / Version source](https://github.com/abnste/mewu_ai/tree/v1.0.0-preview.2) · [MPL-2.0](https://github.com/abnste/mewu_ai/blob/v1.0.0-preview.2/LICENSE)。第三方许可保留在包内。 / Third-party notices are included in the packages.
