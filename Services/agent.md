# Hermes 启动兼容

- Windows Hermes GUI 能对话、CLI `serve` 能启动，并不证明第三方 GUI 启动环境相同。喵呜AI 优先按所发现启动器对应的虚拟环境，用可信本地 `python.exe -m hermes_cli.main` 启动；不得从 PATH 任取系统 Python，也不得混用 `venv` 与 `.venv`。旧纯启动器布局继续兼容。
- 子进程清理外部 PYTHONHOME/PYTHONPATH/VIRTUAL_ENV，只设置 Hermes 自身模块路径；补齐可信自身 Node、Git、虚拟环境目录并启用 UTF-8，不修改用户/系统环境变量或 Hermes 配置。
- 后台使用 `-p default serve` 固定机器根配置，具体人格仍按 RPC profile 选择，不能修改 Hermes 全局默认人格。保留 loopback、随机会话令牌和父进程约束，不使用 `--insecure`，不启用会重复调度 cron 的 HERMES_DESKTOP。
- 进程 Exited 可能早于最后 stderr 回调，启动失败前须有界等待输出收尾；取消必须停止 ready 文件轮询。诊断仅记录白名单错误类别，不存储或展示原始日志、提示词、凭据和环境值。未知退出码不能直接声称安装损坏。
- 参考：Hermes 官方 `apps/desktop/electron/main.ts` 的 createPythonBackend、backend-env.ts，以及微软 Process 异步输出/退出事件说明。朋友电脑问题仍需实际升级验证，不把本机通过当作远端问题已解决。
- 2026-09-04 兼容补丁验收：Release 全量 701 项通过、ProviderSmoke 编译零警告，显式启用的真实 Hermes 测试走通启动、人格列表与模型列表（不发送提示词）。仅生成本地免安装测试包，版本号和 GitHub 正式发行未改动。
