本次更新改进 Windows 下的 Hermes 接入兼容性。 / This update improves Hermes integration on Windows.

- 优先通过 Hermes 自带 Python 启动后台，保持解释器与虚拟环境一致，兼容 venv、.venv 和旧启动器布局。 / Prefer Hermes's bundled Python, keep the interpreter and environment aligned, and support venv, .venv, and legacy launcher layouts.
- 隔离外部 Python 环境污染，补齐 Hermes 自身工具路径并统一 UTF-8 编码。 / Isolate conflicting Python environment variables, include Hermes-managed tool paths, and use UTF-8 consistently.
- 固定后台根配置，所选人格仍通过独立 RPC 上下文使用；不修改 Hermes 配置或全局默认人格。 / Keep backend startup rooted in the default configuration while selecting profiles through RPC, without changing Hermes settings or the global default profile.
- 启动失败时有界等待错误输出，显示脱敏故障类别，不再只给出退出代码；修复取消时的启动等待。 / Drain startup errors with a bounded wait and report privacy-safe failure categories instead of only an exit code; improve cancellation handling.

验证：Release 全量 701 项测试通过；本机真实 Hermes 启动、人格列表与模型列表检查通过。跨电脑实际效果仍需用户环境验证。 / Validation: 701 Release tests passed, plus a live local Hermes startup, profile-list, and model-list check. Behavior on other PCs still needs verification in those environments.

公测模型 / Beta test model: MiniMax M3.

可在“设置 → 关于 → 检查更新”下载安装。 / Install through Settings → About → Check for updates.
