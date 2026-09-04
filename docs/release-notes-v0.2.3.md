本次更新简化 AI 接入设置，并修复 Hermes 兼容性与贴图再次截图的问题。

- 提供商精简为 OpenAI 通用、MiniMax、MiniMax (CN)、火山引擎；只有 OpenAI 通用需要填写 URL，其余只需 Key 和模型，不再要求命名或管理连接。
- 根据所选提供商加载模型，支持手动填写模型 ID；已保存 Key 逐字符掩码显示，各提供商的凭据独立保留。
- 请求参数 JSON 与自定义请求头统一收进默认折叠的高级设置，说明通过小问号悬浮显示。支持 MiniMax M3 `service_tier: priority`，默认不启用；优先服务为标准价格的 1.5 倍。
- 连接测试结果采用统一浅色弹窗；再次截图置顶图片时保留边框和阴影，并限制超大贴图快照的内存占用。
- 改善 Windows Hermes 的启动器、虚拟环境和工具路径兼容性，隔离外部 Python 环境干扰，提供脱敏启动错误提示；不修改 Hermes 全局配置。

本地 Release 全量 736 项测试通过。公测模型：MiniMax M3。Hermes 已通过本机启动、人格与模型列表检查，其他电脑的兼容性仍需实际环境验证。

可在“设置 → 关于 → 检查更新”下载安装。

---

This update simplifies AI setup and fixes Hermes compatibility and recapturing pinned images.

- Choose from OpenAI compatible, MiniMax, MiniMax (CN), or Volcengine. Only OpenAI-compatible endpoints require a URL; the others need just an API key and a model. No connection names or management controls.
- Load models for the selected provider or enter an ID manually. Saved keys appear as masked characters, with credentials kept separate for each provider.
- Request parameters and custom headers now live in a collapsed Advanced settings section, with help on hover. MiniMax M3 supports `service_tier: priority`; it is off by default and costs 1.5× the standard rate.
- Connection-test results use the app's light dialog style. Recaptured pins retain their borders and shadows, with bounded memory use for large snapshots.
- Improve Windows Hermes launcher, virtual-environment, and tool-path compatibility; isolate conflicting Python environment variables and show privacy-safe startup errors without changing Hermes's global settings.

All 736 local Release tests passed. Beta test model: MiniMax M3. Local Hermes startup, profile-list, and model-list checks passed; compatibility on other PCs still needs validation in those environments.

Install through Settings → About → Check for updates.
