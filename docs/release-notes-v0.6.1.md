# 喵呜AI 0.6.1 / MewuAI 0.6.1

本版本基于 v0.6.0，汇总其后合入的 MCP、屏幕实体操作、网页抓取、WorkBuddy 和会话配置更新，并整理 MCP 设置界面。/ This release is based on v0.6.0 and includes the subsequent MCP, screen-entity actions, web crawling, WorkBuddy, and conversation-configuration updates, together with the MCP settings redesign.

## 更新内容 / Changes

- **MCP 外部服务。** 新增 QQ 邮箱 OAuth/PKCE 扫码授权、收件箱上下文与确认后发信；网易邮箱二维码登录和 SMTP 发信；钉钉、飞书截图分享；ima 知识库保存；Obsidian 本地 vault 笔记归档。凭据继续使用本机安全存储。/ **MCP integrations.** Add QQ Mail OAuth/PKCE QR authorization, inbox context and confirmed sending; NetEase QR login and SMTP sending; DingTalk and Feishu screenshot sharing; ima knowledge-base saving; and Obsidian local-vault note archiving. Credentials remain in secure local storage.
- **MCP 设置分组。** MCP 服务按“邮箱”“消息与分享”“知识库与笔记”归类，每个服务以可展开列表项呈现，减少页面拥挤。/ **Grouped MCP settings.** Services are organized into Email, Messaging and sharing, and Knowledge and notes, with each service presented as an expandable list item.
- **屏幕实体识别与操作。** 识别截图中的网址、邮箱和电话号码，并提供打开链接、写邮件或拨号等操作；操作条跟随当前选区，切换、录制、标注和退出时会清理旧状态。/ **Screen entities and actions.** Detect URLs, email addresses and phone numbers in a selection and offer actions such as opening links, composing mail or dialing. The action bar follows the active selection and clears stale state on selection changes, recording, drawing and exit.
- **网页抓取。** 增加基础 HTTP 抓取和可选 Scrapling 抓取，安装探测、代理、取消、子进程输出和失败回收均有界，不阻塞界面。/ **Web crawling.** Add basic HTTP crawling and optional Scrapling crawling with bounded installation detection, proxy handling, cancellation, child-process output draining and cleanup.
- **WorkBuddy 与 AI 配置。** 完善 WorkBuddy ACP 子进程生命周期和双管道排空；增加 Provider 路由、代理设置、模型/可执行文件选择，以及新会话边界和历史恢复。/ **WorkBuddy and AI configuration.** Improve WorkBuddy ACP child-process lifecycle and stdout/stderr draining; add provider routing, proxy settings, model/executable selection, explicit new-conversation boundaries and history recovery.
- **依赖与发布审计。** 增加 QRCoder 的本地 MIT 许可证声明，补充邮件、抓取、屏幕实体和 WorkBuddy 回归覆盖，统一 0.6.1 项目、安装器和发行校验版本。/ **Dependencies and release audit.** Add the local QRCoder MIT notice, regression coverage for mail, crawling, screen entities and WorkBuddy, and synchronize the project, installer and release checks to 0.6.1.

## 验证 / Validation

- Release 构建通过，0 个警告、0 个错误。/ Release build completed with 0 warnings and 0 errors.
- 版本与安装器校验通过；本机 Release 程序启动并响应正常。/ Version and installer checks passed; the local Release application started and responded normally.

## 下载 / Downloads

[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.1/MewuAI-Setup-0.6.1-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.1/MewuAI-Portable-0.6.1-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看。/ [Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.1/MewuAI-Setup-0.6.1-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.1/MewuAI-Portable-0.6.1-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in the GitHub asset details.

完整历史见 [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.1/CHANGELOG.md)。/ See [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.1/CHANGELOG.md) for the full history.
