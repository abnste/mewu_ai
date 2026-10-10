# Mewu AI

[English](README.md)

Mewu 1.0 是 Windows 桌面重制版。快捷键唤起截图空间，对话、截图、文件、标注与交互式 HTML/SVG 结果共处同一界面。

**当前分支为 1.0 预览版。** 0.7.x 稳定版继续保留在 [master](https://github.com/abnste/mewu_ai/tree/master)。预览版从 [Releases](https://github.com/abnste/mewu_ai/releases) 下载，带有预发布标记。

## 当前功能

- 区域截图、绘制、电子黑板、无边框贴图、OCR、翻译、长截图和扫码。
- 多会话、冻结与恢复、Markdown 和公式、可收纳的公开思考、图片及文本附件。
- 屏幕录制，默认电脑声音，支持暂停、裁剪、视频批注及 MP4/GIF 导出。
- Agent 身份、本地 SQLite/FTS5 记忆、可选 Hindsight 接入、运行记录和续答。
- Chat Completions、OpenAI Responses、Anthropic Messages 三种模型接入，以及可从公开 GitHub 安装的模型模板和声明式能力插件。
- 本地 MCP、插件启停/卸载/更新/回退、可配置的 GitHub 社区插件目录。

当前分支支持把截图选区、图片、HTML/SVG 结果、录屏和 TXT 笔记带入黑板，作为独立对象；其他文档不会出现在黑板上。TXT 使用白底编辑区，四角可缩放，编辑保存为副本，保留原文件。截图图片保留原标注并支持悬浮滚轮缩放；HTML 保持交互，视频保留播放、裁切和批注。通过上角的控件拖动对象，拖到底部「删除」后松手移除；橡皮只擦笔迹。绘制色板支持 RGB、HEX 和最近三种颜色；工具条显示或隐藏文字时，按钮均保持相同的正方形尺寸。

绘制、录屏与 AI 标注属于软件内置能力。当前扩展为 JSON 声明式包，不运行下载的任意代码。Hermes、Codex、WorkBuddy、MiniMax Code 外部 Agent 接入，macOS、跨设备同步及专用图片/视频生成接口尚未交付；当前 Agent 还不能完整替代 Hermes 与 Hindsight。原位和视频批注质量取决于实际模型。

## 安装与更新

支持 Windows x64，要求 Windows 10 2004 及以上和 Microsoft WebView2。下载安装器 `MewuAI-Remake-Setup-…-win-x64.exe`，或使用便携 ZIP。

启动后后台检查更新，“设置 → 关于”也可检查、下载并重启。安装前验证版本绑定签名，再保存会话、结束录屏并等待后台任务收尾。固定的发行更新入口不依赖 next、master 或其他分支名称。

旧版 0.7.4 已支持识别未来正式版 1.x 安装器；它会忽略预览版。0.7.3 应先升级到 0.7.4。首次启动自动导入支持的旧版 API 连接，保留旧数据目录；旧版对话历史目前不会转换成新的场景数据库。[升级说明](apps/desktop/README.md)记录了兼容范围。

新安装默认数据目录为用户目录的 `.mewu`；已安装重制版保留当前位置，可在“设置 → 数据”迁移。更新签名与 Windows 代码签名是两回事，本预览版尚未使用 Windows 代码签名证书。

“设置 → 数据”还提供文件、图片和会话的占用统计及独立清理操作。清理保留开放会话、记忆来源和正在引用的素材；未引用的应用内副本保存超过一天后才可清理。会话显示逻辑内容大小，数据库和清理结果显示实际磁盘占用。

## 开发

需要 Node.js 24、Rust stable、Windows C++ 构建工具及 WebView2。

```powershell
npm ci
npm run desktop:dev
node --experimental-vm-modules --test apps/desktop/src/*.test.mjs
cargo test --workspace --locked
npm run build
```

带签名安装包由[发行工作流](.github/workflows/preview-release.yml)构建。本地发行构建须通过 `TAURI_SIGNING_PRIVATE_KEY` 提供妥善保管的私钥，禁止把私钥写进源码或发布目录。浏览器开发预览只提供界面，没有原生截图、录屏和数据保存能力。

项目采用 Rust + Tauri 2 + Solid/TypeScript + SQLite，Windows 界面使用系统 WebView2。核心在 `crates/mewu-core`，原生宿主与界面在 `apps/desktop`。[插件文档](apps/desktop/plugins/README.md)和[社区目录工具](tools/plugin-catalog/README.md)可用于扩展开发。

## 协议

项目继续使用 [MPL-2.0](LICENSE)。[第三方许可全文](apps/desktop/THIRD-PARTY-NOTICES)随安装包和便携包提供，每个版本的发行页面链接对应源码。[问题反馈](https://github.com/abnste/mewu_ai/issues)。
