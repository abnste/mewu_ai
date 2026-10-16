# MewuAI 1.0.1

- 导入图片可在原对话条中直接请求标注、试卷原位作答、公式和表格，也可继续手工绘制。标注独立保存，保留原图，并用于复制、保存、后续引用和置顶。
- 截图、图片和视频标注参数校验失败时，执行记录显示可定位的字段和原因，并将同一诊断反馈给 AI，在原有工具调用限制内修正。旧记录未保存的诊断不会事后补造。
- 改善 MiniMax M3.1 Flash Preview 的绘制参数兼容，并向模型提供完整调用示例，减少目标和字号字段错误。
- 已配置模型时，切换会话不再为了显示对话条读取密钥或弹出钥匙串。真正发送请求时仍会读取和校验凭据；未配置 AI 的截图操作保持原样。
- 修复流式答复重复、绘制完成后的笔迹被置顶对象遮挡，以及待保存的对象位置阻塞退出的问题。
- 图片导入先成为空间对象；图片、视频与黑板预览均可手动置顶，支持滚轮等比缩放。视频操作工具条左对齐，裁切条独立位于下方。
- 仅填写服务域名时，模型列表可尝试常用 API 根路径，并将可用地址回填设置草稿；显式自定义路径保持原样。
- 改善 Windows 允许屏幕共享时的录屏与长截图兼容性，采集结束恢复窗口与输入状态。

## 更新

1.0.1 为新的正式版本，原 1.0.0 发行保留。已有 1.0.0 与预览版可通过「设置 → 关于」检查更新。新版直接读取 GitHub 最新正式发行中的 latest.json，Mac 自动更新归档也属于各自版本发行；后续正式更新不依赖旧 update-channel。安装包与版本签名校验保留，不需要先卸载。

macOS 更新后，系统可能要求重新确认屏幕录制权限。

Windows 0.7.4 仍可从原关于页升级；原 API 配置导入及旧数据的范围见 [1.0.0 说明](./release-notes-1.0.0.md)。旧历史对话尚未转换为新版历史界面。

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

1.0.1 is a new stable release; 1.0.0 remains available. Check for updates in Settings → About. New stable builds read latest.json directly from the latest versioned release, with a versioned macOS updater archive and verified package/version signatures. No uninstall is required. Existing 1.0.0 and preview clients retain their upgrade path. Legacy migration limits remain as described in the [1.0.0 notes](./release-notes-1.0.0.md).

macOS may ask you to confirm screen recording permission again after an update.

AI annotations and answers need human review. Native image formula rendering currently requires Windows. Chat Markdown/KaTeX formulas use the shared frontend renderer. Independent pins remain snapshots; references retain existing ink, and their space image can receive further annotations after the pin closes. Native region automation remains Windows-only; NVIDIA Instant Replay still needs matching hardware testing. The project remains MPL-2.0.
