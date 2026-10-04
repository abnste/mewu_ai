# 覆盖层交互回放

## 隔离回归与输出目录

需要 Windows 和 .NET 10 Windows Desktop。先构建测试工程，再直接运行生成的 EXE；语言与视频回放会启动自身的子进程，因此不要用 `dotnet InteractionHarness.dll` 代替下面的 EXE 调用。

```powershell
dotnet build tests/InteractionHarness/InteractionHarness.csproj -c Release -p:Platform=x64
$harness = (Resolve-Path tests/InteractionHarness/bin/x64/Release/net10.0-windows10.0.19041.0/InteractionHarness.exe).Path
& $harness --verify-pointer-magnifier --english
```

以下入口可以直接从源码目录运行。测试会自动在系统临时目录的 `MewuAI-Replays` 下创建独立任务目录及标记，输出路径写到标准输出；不在源码目录生成回放结果，也不使用正式应用的配置目录。没有控制台时，可在该临时根目录按最近修改时间查找结果。失败返回非零退出码，证据保留供检查。

| 入口 | 覆盖内容 | 输出位置（相对任务目录） |
| --- | --- | --- |
| `--verify-drawing-interaction` | 真实 WPF 绘图、历史、重取景、导出及两行工具栏 | `.codex-build/drawing-interaction` |
| `--verify-localization-startup` | 独立进程内的合成设置保存、加载及中英文启动 | `.codex-build/localization-startup` |
| `--verify-pointer-magnifier` | 合成像素、坐标及放大镜实际渲染 | 任务目录 |
| `--verify-recording-countdown-visual` | 倒计时、序号字形的离屏排版 | 任务目录 |
| `--verify-translation-single-line` | 合成 OCR 行的单行译文与导出排版 | 任务目录 |
| `--verify-screen-entity-scan` | 合成异步识别与抓取生命周期、邮件拒绝和结果不明状态；无窗口，不调用真实 OCR、网络或账号 | 任务目录 |
| `--verify-isolated-video-trim` | 合成音视频裁切、预览、导出及附件构造 | `.codex-build/video-trim` |
| `--render-video-trim` | 时间条的中英文宽窄布局 | 任务目录 |
| `--manual-drawing-desktop` | 人工操作合成绘图覆盖层 | `.codex-build/manual-drawing-desktop/session-<PID>` |
| `--manual-video-trim-desktop` | 人工操作合成视频的裁切与保存 | `.codex-build/manual-video-trim-desktop/session-<PID>` |

支持 `--english` 的视觉/绘图入口可追加该参数；语言启动回放本身覆盖多个语言场景。自动回放不调用真实模型，合成视频回放不会上传视频。WPF 绘图回放会创建测试窗口并可能获得焦点；它检查产品事件和像素，不等于真实鼠标或真实中文输入法验收。不要与正在进行的桌面操作并行运行。

已有自动化仍可从 `MINI_TEMP_ROOT` 下的独立目录运行：只要该目录或根目录内的祖先已有 `.mini-temp` 标记，输出位置保持不变。兼容默认的本机 Mini 临时目录。新用户无需设置这个变量或手工创建标记。测试进程继承的 `MEWU_REPLAY_TEMP_ROOT` 仅用于让子进程识别父任务目录，不是产品设置；输出路径拒绝源码检出目录、正式安装/配置目录及文件系统链接。

两个 `--manual-*` 入口需要明确选择才会打开交互窗口。绘图辅助程序会在内存保存并恢复启动时的剪贴板，原始剪贴板内容不写盘；仅显式导出本测试进程新复制的图片。它还记录有限快捷键与焦点状态，不记录输入正文。视频辅助程序使用合成素材，有自动关闭期限。日志、JSON、PNG 和媒体证据均留在临时任务目录，不应提交或随安装包发布。若绘图会话报告剪贴板恢复失败，应保留该进程中的备份并按状态重试恢复，不要直接结束进程丢弃备份。

## 逻辑子集与源码连接检查

`dotnet test tests/MewuAI.MaintenanceChecks/MewuAI.MaintenanceChecks.csproj -c Release` 可单独运行跨平台的几何/标注逻辑子集。该工程直接链接产品源码与部分单测，`PlatformShim.cs` 提供测试用的 `System.Windows` 简化类型；通过只代表这些逻辑断言通过，不能替代 Windows/WPF 的渲染、输入、生命周期或原生媒体验收。它不在默认解决方案及当前发行工作流的单测入口内。

`python tests/source-contracts/capture-interaction-contract.py` 只检查指定源码方法之间的连接关系，对代码结构变化敏感；它不是 Windows 事件或实际交互测试。测试源码可以公开保留，内部 `agent.md`、本机运行记录和生成物继续忽略。产品工程排除了 `tests/**`，不要把测试程序复制进正式发布目录。

## 既有专项入口

下面的较早入口仍按各自说明选择输出目录和外部依赖；上述自动临时目录行为仅适用于表中的入口。

翻译专项：Release 构建后运行 `dotnet tests/InteractionHarness/bin/x64/Release/net10.0-windows10.0.19041.0/InteractionHarness.dll --teaching --verify-translation`，用实际 PP-OCRv6 识别合成双栏材料，再检验原位锚点和导出像素。加 `--live-translation` 会使用当前默认翻译 API，发送同一合成文本比较并发与串行耗时；不发送桌面、不修改设置。报告及合成图仅保存在 `.codex-build/translation`。

窗口问题回归：Release 构建后运行 `dotnet tests/InteractionHarness/bin/x64/Release/net10.0-windows10.0.19041.0/InteractionHarness.dll --teaching --verify-window-issues`。仅显示合成窗口，检查 Issue #3 的主界面/设置页原生防捕获标记，以及 Issue #4 在普通/教学两种模式下的连续截图、贴图区域鼠标命中、实际贴图命令和退出后的贴图状态。结果写入忽略目录 `.codex-build/issue-3-4/window-replay.json`，失败退出码为 1；NVIDIA 硬件仍需单独验收。

仅本机 Debug 验收：`dotnet run --project tests/InteractionHarness/InteractionHarness.csproj -c Debug -p:Platform=x64`。
追加 `-- --english` 查看英文布局。Esc 退出，五分钟自动关闭。

回放实际覆盖层的合成长回答，检查输入条向上展开、历史入口、鼠标滚动暂停跟随和“回到最新回复”。背景为合成卡片，不加载真实设置或磁盘历史、不发送网络请求、不启动录屏。仅设置 Debug 已有防捕获 QA 开关；不得加入正式发布。

先退出正常应用再启动，避免和用户正在操作的覆盖层混淆。

运行 `dotnet tests/InteractionHarness/bin/x64/Debug/net10.0-windows10.0.19041.0/InteractionHarness.dll --verify-lifetime` 执行真实覆盖层的资源回收契约检查：创建/删除 60 个区域，在 50 步历史中保留 25 个可撤销区域，验证 redo 与在途请求保护，历史淘汰后确认区域和租约归零。结果仅写入忽略的 `.codex-build/interaction-lifetime.json`，不生成真实媒体文件。

性能回放：Release 构建后以 `--teaching --benchmark` 记录修改前样本，追加 `--after` 记录修改后样本；合成 160 次更新、9010 字正文，结果仅在 `.codex-build/answer-before.json` 和 `answer-after.json`。测量期间不要同时构建、跑测试或做桌面自动化，避免 CPU/GPU 竞争污染结果。该回放展示真实覆盖层并在完成时关闭，不能与用户日常截图混用。

滚动条箭头回归：Release 构建后运行 `dotnet tests/InteractionHarness/bin/x64/Release/net10.0-windows10.0.19041.0/InteractionHarness.dll --teaching --verify-answer-alignment`。真实覆盖层渲染长回答，比较滑块与箭头的实际中心坐标（误差不超过 0.1 DIP），并验证箭头显隐不改变正文及对话条边界；失败返回非零退出码。
