# 更新历史 / Changelog

每个正式版本对应自己的源码标签、安装包和双语发行说明。后续功能写入新版本；旧版本说明保留当时发布内容。/ Each release has its own source tag, packages, and bilingual notes. Later changes belong to later releases.

## 0.3.4 — 对话图片与翻译排版 / Reply images and translation layout

- 修复 AI 回复图片不显示，接入 Hermes 本地图片、MEDIA 引用及图片工具结果。/ Fixed missing reply images, including local images, MEDIA references, and image-tool results from Hermes.
- 去掉图片的灰色底板和程序标签，复制时保留说明、文字与表情。/ Removed gray image backdrops and app-added labels while preserving descriptions, text, and emoji when copying.
- 修复长译文及双栏译文重叠，统一显示、选择和导出布局。/ Fixed overlapping translations across lines and columns, with consistent display, selection, and export layouts.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.4.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.4) · [v0.3.3 → v0.3.4](https://github.com/abnste/mewu_ai/compare/v0.3.3...v0.3.4)

## 0.3.3 — 视频置顶、思考光效与开源许可 / Pinned videos, activity glow, and license notices

- 修复带标注视频置顶后的闪退。/ Fixed crashes after pinning annotated videos.
- 新增可开关、可调 RGB 颜色的底部思考光效，修复任务栏区域截断。/ Added a configurable AI activity glow that reaches the full display edge.
- 统一历史与最新回复的复制菜单，修复阴影裁切，保留完整内容和表情。/ Unified reply copy menus, fixed clipped shadows, and preserved full text and emoji.
- 关于页新增第三方组件与完整许可的离线查看入口。/ Added offline access to third-party components and full license notices in About.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.3.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.3) · [v0.3.2 → v0.3.3](https://github.com/abnste/mewu_ai/compare/v0.3.2...v0.3.3)

## 0.3.2 — 视频时间、对话输入与中文显示 / Video timing, input, and Chinese text

- 修复 MiniMax 视频时间压缩为零点几秒，以及末尾帧不完整引发的 HTTP 400；保留原视频。/ Fixed compressed MiniMax timestamps and HTTP 400 errors caused by incomplete final frame groups, preserving the original video.
- 修复视频播放/暂停状态、对话条弹出焦点、录制后直接输入和 Ctrl+C 复制回答。/ Fixed playback controls, input focus when the bar appears, typing after recording, and Ctrl+C copying.
- 统一中文回复字体，默认简体中文，保留明确的语言要求、引用与代码。/ Unified Chinese reply fonts and defaulted Chinese answers to Simplified Chinese while preserving explicit language requests, quotations, and code.
- 新增跨截图关联箭头，改善多选区工具条显隐与逐行翻译。/ Added cross-screenshot connection arrows and improved selection toolbars and line-by-line translation.

- 修复首轮回答显示后继续提问仍长时间停在“AI 正在分析”的问题；后台补标会被安全取消并让出请求。/ Fixed follow-up questions remaining stuck on “AI is analyzing” after the first answer; background annotation repair now yields to the new request safely.

- 修复多区域提问期间工具条只响应一个选区的问题，等待回答和后台补标时也能悬停切换。/ Fixed toolbars responding to only one region during multi-region questions, including while waiting for an answer or additional annotations.

- 截图框选、移动或缩放完成后自动聚焦对话输入，鼠标悬停不再打断刚开始的输入。/ The conversation input receives focus after selecting, moving or resizing a screenshot, and hovering no longer interrupts typing.

- 教学演示默认开启，移除截图左上角的共享状态标识；仍可在设置中关闭。/ Teaching mode is on by default, without a sharing-status badge on the capture overlay. It can still be turned off in Settings.

- 教学演示模式支持录屏和长截图，控制条与预览自动避开采集区。全屏没有空位时可用 F8 停止/完成，倒计时中可取消。/ Teaching mode now supports recording and scrolling capture. Controls and previews stay outside the capture area; F8 stops or finishes capture and cancels the countdown.

[完整双语说明 / Full notes](./docs/release-notes-v0.3.2.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.2) · [v0.3.1 → v0.3.2](https://github.com/abnste/mewu_ai/compare/v0.3.1...v0.3.2)

## 0.3.1 — 视频请求诊断 / Video request diagnostics

- 视频请求失败时显示实际渠道、可识别的失败原因、服务代码和安全的追踪编号，不再只显示笼统的 HTTP 状态。
- 对 MiniMax 返回的“视频内容被拒绝”也给出明确提示，避免误导用户反复压缩一个本身未超限的文件。
- 错误处理不会回显服务端可能包含的提示词、媒体数据或认证信息。
- 统一 API 与 MiniMax Code 的非成功响应处理，并补齐视频 422 的安全回归测试。
- 原位翻译可识别常见的 JSON 包装和空白 OCR 行；未按原行数返回时会自动重试一次，再给出可操作的提示。

[完整双语说明 / Full notes](./docs/release-notes-v0.3.1.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.1) · [v0.3.0 → v0.3.1](https://github.com/abnste/mewu_ai/compare/v0.3.0...v0.3.1)

## 0.3.0 — 多渠道 AI 与模型切换 / Multiple AI channels

- 新增 ChatGPT Work / Codex、WorkBuddy、MiniMax Code 桌面接入；保留 API、Hermes。
- 多 API 接入列表、配置共存、单层模型菜单、记住上次渠道和模型。
- 修复纯文字视觉提示、原始 JSON 回答、WorkBuddy 思考/连接等待与模型选项兼容。
- 统一 AI 设置、主页渠道状态、快捷键清空和模型弹层布局。
- 正式采用 MPL-2.0，补齐社区规范；README 使用 GIF，下载资产仅安装 EXE 与便携 ZIP。

[完整双语说明 / Full notes](./docs/release-notes-v0.3.0.md) · [Release](https://github.com/abnste/mewu_ai/releases/tag/v0.3.0) · [v0.2.6 → v0.3.0](https://github.com/abnste/mewu_ai/compare/v0.2.6...v0.3.0)

## 历次正式发布 / Previous published releases

以下版本已在 GitHub 发布安装包。原发行说明通过版本标签永久定位，仓库内文档保留原文。/ These versions have published packages; tagged notes preserve the original release text.

| 版本 / Version | 主要内容 / Highlights | 原发行说明 / Original notes |
| --- | --- | --- |
| [0.2.6](https://github.com/abnste/mewu_ai/releases/tag/v0.2.6) | 标注对象直接编辑、RGB 复制、录屏电脑/麦克风音频、MP3 导出、Hermes 安装后启动、历史复制和高 DPI 菜单 / Annotation handles, RGB copying, recording audio, MP3 export, Hermes startup, history and DPI fixes | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.6/docs/release-notes-v0.2.6.md) |
| [0.2.5](https://github.com/abnste/mewu_ai/releases/tag/v0.2.5) | 教学共享、增量回答与交互性能、滚动箭头、设置自动检查更新、录屏测试时序修复 / Teaching mode, streaming performance, scroll arrows, automatic update checks, recording test timing | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.5/docs/release-notes-v0.2.5.md) |
| [0.2.3](https://github.com/abnste/mewu_ai/releases/tag/v0.2.3) | Hermes 启动兼容、API 提供商配置、请求参数和贴图阴影 / Hermes startup, API configuration, request parameters and pinned-image shadows | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.3/docs/release-notes-v0.2.3.md) |
| [0.2.2](https://github.com/abnste/mewu_ai/releases/tag/v0.2.2) | 自包含包还原与慢机器录屏取消等待，包含 0.2.0 / 0.2.1 的功能 / Self-contained restore and recording cancellation, including 0.2.0 / 0.2.1 changes | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.2.2/docs/release-notes-v0.2.2.md) |
| [0.1.0](https://github.com/abnste/mewu_ai/releases/tag/v0.1.0) | 稳定性与隐私、马赛克坐标、结构化回答、OCR/翻译、历史抽屉 / Stability, privacy, mosaic coordinates, structured answers, OCR/translation and history drawer | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.1.0/docs/release-notes-v0.1.0.md) |
| [0.0.11](https://github.com/abnste/mewu_ai/releases/tag/v0.0.11) | 关于页与设置导航裁切 / About page and settings navigation clipping | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.11/docs/release-notes-v0.0.11.md) |
| [0.0.10](https://github.com/abnste/mewu_ai/releases/tag/v0.0.10) | 上传按钮、流式布局、Hermes 状态、批注、翻译与长截图 / Upload button, streaming layout, Hermes status, annotations, translation and scrolling capture | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.10/docs/release-notes-v0.0.10.md) |
| [0.0.9](https://github.com/abnste/mewu_ai/releases/tag/v0.0.9) | Release 资产补传 / Release asset upload recovery | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.9/docs/release-notes-v0.0.9.md) |
| [0.0.8](https://github.com/abnste/mewu_ai/releases/tag/v0.0.8) | 安装器版本同步、思考与标注布局、附件引用 / Installer version sync, reasoning and annotation layout, attachment references | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.8/docs/release-notes-v0.0.8.md) |
| [0.0.4](https://github.com/abnste/mewu_ai/releases/tag/v0.0.4) | 智能框选、长截图、马赛克与统一图像/视频批注 / Window selection, scrolling capture, pixelation and image/video annotations | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.4/docs/release-notes-v0.0.4.md) |
| [0.0.1](https://github.com/abnste/mewu_ai/releases/tag/v0.0.1) | 首个 Windows x64 公开版本 / First public Windows x64 release | [中英 / ZH–EN](https://github.com/abnste/mewu_ai/blob/v0.0.1/docs/release-notes-v0.0.1.md) |

## 保留的发布准备记录 / Retained release preparation records

以下文档和标签属于准备或失败构建，没有对应的正式 Release 下载；不能把它们当成已成功发布。/ The following preparation or failed-build records have no corresponding published Release downloads.

- [0.2.4](./docs/release-notes-v0.2.4.md)：正式构建被 GIF 录屏测试拦截，改由 0.2.5 发布。
- [0.2.1](./docs/release-notes-v0.2.1.md)、[0.2.0](./docs/release-notes-v0.2.0.md)：还原/录屏测试未完成发布，功能进入 0.2.2。
- [0.0.7](./docs/release-notes-v0.0.7.md)、[0.0.3](./docs/release-notes-v0.0.3.md)、[0.0.2](./docs/release-notes-v0.0.2.md)：保留的早期发布准备文档。

## 2026-09-07 文档与附件修正 / Documentation and asset correction

此前把 v0.2.6 之后的文档清理追加到 v0.2.6 Release 是一次发布记录错误，现已恢复原始功能说明，新功能按 v0.3.0 独立归档；旧标签和安装包未移动或替换。v0.2.6 的 SHA256SUMS.txt 在 2026-09-07 被移除，而不是发布时没有提供。用户从 v0.2.5 及更早版本升级时应手动下载安装器，避免旧更新器找不到校验文件。

An earlier edit incorrectly added later documentation cleanup to the v0.2.6 release. Its original feature notes are restored, and subsequent features are recorded under v0.3.0. Existing tags and binaries are unchanged. The v0.2.6 checksum attachment was removed on 2026-09-07; it was present at the original release. Users of v0.2.5 or earlier should download the installer manually because those updaters require the old checksum file.
