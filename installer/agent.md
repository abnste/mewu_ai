# 安装后启动交接

- Windows 11 26100 + Inno Setup 6.7.1 的实测中，直接启动的应用继承安装器 `ProcessRedirectionTrustPolicy=1`。Hermes 的 uv trampoline 随后访问普通用户创建的 Python junction 时返回 Windows 448 并退出 1。用户手动重开成功不能视为修复。
- `MewuAI.iss` 的交互安装和静默更新都通过 `{win}\explorer.exe` 交接应用启动；应用绝对路径必须独立加双引号，保持 `runasoriginaluser`。不要关闭安装器 RedirectionGuard，不修改用户 junction、系统策略或 Hermes 配置。
- 已验证直接 Exec、仅 `runasoriginaluser`、仅 ShellExec 都不足以解决此环境的问题；桌面交接后的应用策略为 0，真实设置页成功返回人格/模型目录。Inno 文档称保护不继承，与此 Windows 构建的实测有差异，应以完整进程链路验收为准。
- 复测需要关闭现有主实例，再从安装器启动软件，否则单实例唤醒会复用旧进程、掩盖继承状态。检查真实设置页“测试连接”，不能仅运行终端 Python 或独立 ProviderSmoke。复测不得发送提示词、改变用户 Profile/模型/凭据设置。
- `PublishDir` 可以在编译时显式覆盖，用于隔离本地验证包；默认仍是 CI 的 `artifacts/release/win-x64`。本地修复包不可覆盖已有 GitHub 版本标签或资产。

官方参考：
- https://jrsoftware.org/ishelp/topic_setup_redirectionguard.htm
- https://jrsoftware.org/ishelp/topic_runsection.htm
- https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocessmitigationpolicy
- https://devblogs.microsoft.com/oldnewthing/20190425-00/?p=102443
