# MewuAI 1.0.1

- 导入图片可在原对话条中直接请求标注、试卷原位作答、公式和表格，也可继续手工绘制。标注独立保存，保留原图，并用于复制、保存、后续引用和置顶。
- 截图、图片和视频标注参数校验失败时，执行记录显示可定位的字段和原因，并将同一诊断反馈给 AI，在原有工具调用限制内修正。旧记录未保存的诊断不会事后补造。
- 改善 MiniMax M3.1 Flash Preview 的绘制参数兼容，并向模型提供完整调用示例，减少目标和字号字段错误。
- 已配置模型时，切换会话不再为了显示对话条读取密钥或弹出钥匙串。真正发送请求时仍会读取和校验凭据；未配置 AI 的截图操作保持原样。
- 修复流式答复重复、绘制完成后的笔迹被置顶对象遮挡，以及待保存的对象位置阻塞退出的问题。
- 图片导入先成为空间对象；图片、视频与黑板预览均可手动置顶，支持滚轮等比缩放。视频操作工具条左对齐，裁切条独立位于下方。
- 仅填写服务域名时，模型列表可尝试常用 API 根路径，并将可用地址回填设置草稿；显式自定义路径保持原样。
- 改善 Windows 允许屏幕共享时的录屏与长截图兼容性，采集结束恢复窗口与输入状态。
- 更新直接读取 GitHub 正式发行并选择当前系统的安装包，不再依赖额外更新清单文件。关于页仅保留版本和更新操作，移除“构建”行。
- 插件按系统和本机能力禁用不支持的操作，通用 JavaScript 插件仍使用同一个包。修复设置中插件项目、发行链接无法打开的问题。

## 更新

1.0.1 为新的正式版本，原 1.0.0 发行保留。新版通过「设置 → 关于」直接检查 GitHub 最新正式发行并下载对应系统的安装包，保留安装包与版本签名校验，不依赖独立更新清单或旧渠道发行。

macOS 更新后，系统可能要求重新确认屏幕录制权限。

旧 0.7.4、1.0.0 和预览版可能仍写死旧更新地址或安装包名，请从本发行安装一次，无需先卸载；之后使用新版自动更新。原 API 配置导入及旧数据范围见 [1.0.0 说明](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0)。旧历史对话尚未转换为新版历史界面。

## 当前范围

支持 Windows x64、macOS ARM64 和 Linux x64，协议保持 MPL-2.0。AI 标注取决于模型的识读与工具调用结果，试卷答案仍需人工核对。当前原生图片公式渲染仅支持 Windows；聊天中的 Markdown/KaTeX 公式使用三平台共享渲染。独立置顶图片保持快照；引用时保留已有批注，关闭置顶后可在空间中的引用图片上继续原位标注。原生区域自动操作插件目前仅支持 Windows；NVIDIA 即时重放仍缺匹配硬件实测。执行记录用于 AI 任务追踪，不是全程序实时诊断日志。

## English

- Imported images support in-place AI annotations, answers, formulas and tables through the ordinary conversation bar, alongside manual drawing. Annotations are stored separately from the original and used by image export, references and pinning.
- Invalid screenshot, image and video annotation arguments expose available bounded field diagnostics in execution details and return the same diagnostics to the AI within the existing tool limits. Existing records without diagnostics are not retrospectively changed.
- Improves drawing argument compatibility for MiniMax M3.1 Flash Preview and supplies complete invocation examples to reduce target and font-size errors.
- Conversation visibility uses model configuration metadata. Switching scenes no longer reads credentials or prompts the keychain; actual requests still read and validate credentials.
- Fixes repeated streamed replies, saved ink being covered by pinned objects, and exit being blocked by pending object geometry.
- Images are imported as space objects. Images, videos and board previews support explicit pinning and proportional wheel zoom. Video actions align left above the picture; trimming stays below it.
- Model discovery can try common API roots for a bare service origin, while preserving explicit custom paths. Windows shared recording and scrolling capture restore window and input state after capture.
- Updates read the latest stable GitHub Release directly and select the package for the current system. About shows the version and update controls without the build row.
- Unsupported plugin operations are disabled per system and native capability; portable JavaScript extensions still use one package. Plugin project and release links now open from Settings.

1.0.1 is a new stable release; 1.0.0 remains available. Settings → About reads GitHub Releases directly, with package and version verification and no separate feed file. Earlier 0.7.4, 1.0.0 and preview installations may request old endpoints or package names: install this release once without uninstalling, then use the new automatic updates. Legacy data migration limits remain as described in the [1.0.0 notes](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0).

macOS may ask you to confirm screen recording permission again after an update.

AI annotations and answers need human review. Native image formula rendering currently requires Windows. Chat Markdown/KaTeX formulas use the shared frontend renderer. Independent pins remain snapshots; references retain existing ink, and their space image can receive further annotations after the pin closes. Native region automation remains Windows-only; NVIDIA Instant Replay still needs matching hardware testing. The project remains MPL-2.0.
