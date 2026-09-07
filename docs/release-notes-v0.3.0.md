本次更新汇总 v0.2.6 之后的多渠道 AI 接入、模型选择重构和对话修复。应用、安装器、源码标签与下载包统一为 0.3.0。

### 新增 AI 接入

- **ChatGPT Work / Codex：** 通过官方本机 `codex app-server` 接入，沿用官方客户端登录；设置页读取真实模型与思考程度，支持文字、图片及交给 Agent 自行处理的视频。不提供 Chat 页，用量按官方账号规则计算。
- **WorkBuddy：** 通过桌面客户端自带 CLI 的官方 ACP 接口接入，复用本机登录并读取真实模型和思考选项。视频由 Agent 使用本机已有工具分析。
- **MiniMax Code 桌面版：** 增加独立配置页，支持发现桌面登录、打开官方客户端、刷新状态和测试连接，无需另外安装或登录 CLI。当前通过桌面会话兼容接入，和 Codex / WorkBuddy 的 Agent 工具接口不同；不宣称为官方公开第三方 SDK。
- API、Hermes、Codex、WorkBuddy、MiniMax Code 使用统一的顶部配置标签与表单布局。移除尚未适配的 OpenClaw、Claude Code 占位页；本版没有豆包工作桌面接入。

### 多渠道与模型切换

- API 接入改为列表，可保存多个接入点、模型和独立凭据；切换配置页保留未保存草稿，保存后保留各渠道配置。
- 取消用设置页单选项控制所有渠道的旧逻辑；已配置且满足当前使用条件的渠道都进入对话选择列表，兼容旧 WorkBuddy 开关关闭但已有配置的情况。
- 模型入口位于对话条上传按钮之后，使用同款圆形图标。点击直接展开一层列表，点选即切换，当前项显示勾号；支持滚动和键盘导航，修复弹层边框、阴影裁切。
- API 列表只列实际配置的接入点与模型，避免把推荐模型或账户目录当成已配置模型。
- 默认使用上一次选择的渠道及其模型，重启后保留；主页状态随实际渠道显示，修复已选 Codex 却仍显示 MiniMax 的问题。不同渠道和模型的历史分开保存。

### 对话与连接修复

- 修复没有截图、录屏或上传附件时仍附带识图和视觉批注提示词的问题；文字请求不自动创建隐式全屏截图。
- 修复 Agent 回答直接显示 `annotationProtocol` 等原始 JSON；统一提取正文并校验可执行批注。
- WorkBuddy「测试连接」改为协议握手、会话建立和模型/思考选项检查，不发送推理对话；等待有独立的 30 秒上限，超时明确结束。该检查通过不等于所有模型推理能力已验证。
- 修复 WorkBuddy 思考迟迟不结束及动态模型/思考选项被误判不可用；完成、取消、失败按当前请求处理，不把工具活动或仅有思考的内容当作最终答案。
- API 与 MiniMax Code 的连接结果改为设置页内的绿色成功文字，统一各页状态呈现。两者手动测试仍发送简短验证消息，可能使用服务额度。

### 界面、许可与发布

- 主页移除最小化按钮，隐藏到托盘入口改用关闭图标；全局截图快捷键支持按 Delete 清空并停用。
- 项目自有源码正式采用 MPL-2.0，发布包携带 LICENSE 与 SOURCE.md；具体许可范围和第三方条款见仓库对应文件。
- 补齐社区行为准则、贡献指南、安全策略、Issue / PR 模板；README 更新全部当前渠道和使用方法，并新增完整版本索引。
- README 演示保留 GIF，移除仓库源演示 MP4。新 Release 只附安装 EXE 和便携 ZIP，校验使用 GitHub 资产原生 SHA-256，不附演示视频或 SHA256SUMS.txt。

### 升级与兼容范围

- v0.2.5 及更早客户端依赖旧校验文件，自动更新可能失败；请从本仓库 Release 手动下载安装器升级。v0.2.6 起支持 GitHub 原生 digest。
- Codex / WorkBuddy 已有本机文字、合成图片和合成视频验证记录；这不代表所有账户、模型或其他电脑均通过。Agent 视频能力取决于本机已有工具，喵呜AI不捆绑或自动安装视频解码工具。
- MiniMax Code 桌面会话、客户端版本和所选模型会影响可用性；未登录、会话过期或额度不足时需在官方客户端处理。

[全部提交变化](https://github.com/abnste/mewu_ai/compare/v0.2.6...v0.3.0) · [完整版本记录](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)

---

This release brings together the AI integrations, channel picker redesign, and conversation fixes developed after v0.2.6. The application, installer, source tag, and packages use version 0.3.0.

### AI integrations

- **ChatGPT Work / Codex:** connect through the official local `codex app-server`, reuse the official login, and read real models and reasoning options. Text, images, and videos handed to the Agent are supported. There is no Chat tab; usage follows the account's official rules.
- **WorkBuddy:** connect through its bundled CLI's official ACP interface, reuse the local login, and read real model and reasoning options. The Agent inspects videos using tools already installed on the PC.
- **MiniMax Code desktop:** add a settings page to discover the desktop session, open the official client, refresh status, and test a connection without a separate CLI installation or login. This desktop-session compatibility path is different from the Codex / WorkBuddy Agent interfaces and is not presented as an official public third-party SDK.
- Unify the API, Hermes, Codex, WorkBuddy, and MiniMax Code settings layouts. Remove the unimplemented OpenClaw and Claude Code placeholders. Doubao Work desktop is not included in this release.

### Channels and model selection

- Save multiple API endpoints with separate models and credentials. Preserve configuration drafts when navigating settings and retain each channel's saved configuration.
- Make configured, usable channels available together instead of enabling only the selected settings tab. Recognize saved WorkBuddy configurations even when a legacy enable flag is off.
- Place a round model button immediately after Upload. Open a single scrollable list, switch with one click, mark the current item, support keyboard navigation, and fix clipped popup borders and shadows.
- Show only configured API endpoints/models, rather than treating suggested or account-listed models as configured entries.
- Remember the last channel/model across restarts, display it on the home page, and keep history scoped to its channel/model. Fix the home page showing MiniMax after selecting Codex.

### Conversation and connection fixes

- Keep text-only messages free of automatic screen attachments and visual-analysis instructions.
- Parse Agent responses into readable answers instead of displaying raw annotation-protocol JSON; validate executable annotations separately.
- Check WorkBuddy through ACP initialization, session creation, and model/reasoning configuration with a separate 30-second timeout. The check sends no inference prompt and does not certify every model's inference capability.
- Fix lingering WorkBuddy thinking and incorrect rejection of dynamic model/reasoning options. Distinguish completed answers from tool activity, reasoning-only output, cancellation, and failure.
- Show consistent inline connection results, including green success text for API and MiniMax Code. Their manual tests still send a short verification message and may use service quota.

### Interface, licensing, and distribution

- Replace the home-page tray-hiding icon with Close, remove Minimize, and allow Delete to clear and disable the capture shortcut.
- Adopt MPL-2.0 for project-owned source and include LICENSE / SOURCE.md in packages. See the repository for scope and separate third-party terms.
- Add community, contribution, security, issue and PR guidance; update both READMEs and add a complete release-history index.
- Keep the README GIF and remove the source demo MP4. Releases attach only the installer and portable ZIP, using GitHub's native asset SHA-256 without a separate checksum file or demo video.

### Upgrade and compatibility

- v0.2.5 and earlier updaters require the old checksum file and may fail to update automatically. Download the installer manually from this repository's Releases. Native GitHub digests are supported starting with v0.2.6.
- Existing local Codex / WorkBuddy text, synthetic-image, and synthetic-video checks do not verify every account, model, or PC. Agent video analysis depends on locally installed tools; MewuAI does not bundle or install decoders.
- MiniMax Code availability depends on its desktop session, client version, and selected model; resolve expired login or quota issues in the official client.

[Full comparison](https://github.com/abnste/mewu_ai/compare/v0.2.6...v0.3.0) · [Release history](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)
