# 喵呜AI 0.5.2 / MewuAI 0.5.2

- 新增 OpenAI Chat、OpenAI Responses 和 Anthropic Messages API 格式兼容，并保留现有 OpenAI-compatible Provider 行为。
- 新增系统代理、直连和自定义代理设置；代理地址校验不允许携带账号密码或隐式路径。
- 新增历史会话档案，按 Provider、模型和会话隔离，支持从覆盖层恢复历史会话。
- 修复流式响应中非法 JSON 结构被误判为完成的问题，补充 Responses 与 Anthropic 事件解析和恢复测试。
- 保留原作者 `shuziyuxingxing-stack` 的 Provider、代理和历史会话贡献署名。

下载：[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.2/MewuAI-Setup-0.5.2-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.2/MewuAI-Portable-0.5.2-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看，不附带单独校验文件。

---

- Adds OpenAI Chat, OpenAI Responses and Anthropic Messages API compatibility while preserving existing OpenAI-compatible Provider behavior.
- Adds system proxy, direct connection and custom proxy settings; proxy URLs reject embedded credentials and implicit paths.
- Adds conversation history archives isolated by Provider, model and session, with in-overlay history restoration.
- Fixes malformed streaming JSON being mistaken for a completed response, with additional Responses and Anthropic event parsing and recovery coverage.
- Preserves attribution for the original Provider, proxy and conversation-history contribution by `shuziyuxingxing-stack`.

Downloads: [installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.2/MewuAI-Setup-0.5.2-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.2/MewuAI-Portable-0.5.2-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in GitHub asset details; no separate checksum file is included.
