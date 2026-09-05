# Hermes 启动兼容

- 2026-09-05 性能审查：`BufferedAiStreamProgress` 在 Provider 线程合并已归一化增量，首批排一次 Background 回调，之后约 80ms 刷新；每个请求独享实例，完成前 Flush，取消/替换/关闭后丢弃未渲染批次，渲染异常交回请求失败路径。不得恢复逐 token `Progress<T>` UI 排队或在每次增量中强制 `UpdateLayout`。
- `SelectionImageCache` 按冻结桌面源对象和物理裁剪矩形复用 WIC 裁剪；样式/引用变化不重建图像，桌面更换、选区变化或资源释放时失效。不得把缓存当作可导出的标注快照。
- JSONL 历史加载最多读取末尾 8 MiB；从字节边界开始时先跳过不完整首行，再解码 UTF-8，避免截断中文/emoji 让近期记录整体失效。原历史文件不裁剪不重写；仍逐条严格解析 JSON 并执行条数、单条长度和 Provider/Profile 隔离。
- AI 图片编码在复制出最终附件后必须清零 MemoryStream 的原始缓冲；每次 PNG/JPEG 编码完成都检查取消，再转移缓冲所有权。Dispose MemoryStream 不等于清除其中的屏幕内容。

- `PinnedVisualSnapshotRenderer` 用显式 VisualBrush 视口渲染贴图完整外观；快照最长边 8192px、总像素 1600 万，超大贴图下采样但不改变实际物理目标边界或限制用户缩放。100%、150%、200% DPI 的透明边距、阴影及负坐标桌面合成均有像素回归测试。2026-09-04 本轮 Release 全量 735 项通过，ProviderSmoke 编译零警告。

- Windows Hermes GUI 能对话、CLI `serve` 能启动，并不证明第三方 GUI 启动环境相同。喵呜AI 优先按所发现启动器对应的虚拟环境，用可信本地 `python.exe -m hermes_cli.main` 启动；不得从 PATH 任取系统 Python，也不得混用 `venv` 与 `.venv`。旧纯启动器布局继续兼容。
- 子进程清理外部 PYTHONHOME/PYTHONPATH/VIRTUAL_ENV，只设置 Hermes 自身模块路径；补齐可信自身 Node、Git、虚拟环境目录并启用 UTF-8，不修改用户/系统环境变量或 Hermes 配置。
- 后台使用 `-p default serve` 固定机器根配置，具体人格仍按 RPC profile 选择，不能修改 Hermes 全局默认人格。保留 loopback、随机会话令牌和父进程约束，不使用 `--insecure`，不启用会重复调度 cron 的 HERMES_DESKTOP。
- 进程 Exited 可能早于最后 stderr 回调，启动失败前须有界等待输出收尾；取消必须停止 ready 文件轮询。诊断仅记录白名单错误类别，不存储或展示原始日志、提示词、凭据和环境值。未知退出码不能直接声称安装损坏。
- 参考：Hermes 官方 `apps/desktop/electron/main.ts` 的 createPythonBackend、backend-env.ts，以及微软 Process 异步输出/退出事件说明。朋友电脑问题仍需实际升级验证，不把本机通过当作远端问题已解决。
- 2026-09-04 兼容补丁验收：Release 全量 701 项通过、ProviderSmoke 编译零警告，显式启用的真实 Hermes 测试走通启动、人格列表与模型列表（不发送提示词）。仅生成本地免安装测试包，版本号和 GitHub 正式发行未改动。

# Provider 模型目录

- ProviderPresetPolicy 只做界面厂商与现有 Type/BaseUrl 的映射，不增加会让旧配置失效的必填字段；精确地址匹配，非标准 MiniMax 地址仍保留为自定义配置及原有 MiniMax 协议。
- 提供商目录只保留四项；国内 MiniMax 的稳定内部 ID 仍为 MiniMax、国际仍为 MiniMaxGlobal，不能因显示名改为 MiniMax (CN)/MiniMax 而交换既有凭据。旧 OpenAI 官方和第三方兼容地址统一落入 OpenAI 通用（内部 Custom），仅此项要求 URL；标准厂商隐藏固定 URL。界面不再要求命名或管理连接。
- ProviderModelCatalogService 统一读取兼容 `/models` 的 `data[].id`。禁止跨域重定向和 Cookie，拒绝同时发送 API Key 与认证 Header；读取无 Content-Length 的流时同样逐块执行 2 MiB 上限，最多保留 4096 个模型，错误不暴露远端响应体。火山沿用对话模型过滤；MiniMax 等不得套用火山前缀白名单。
- 官方参考：MiniMax `/docs/api-reference/models/openai/list-models` 与微软 WPF ComboBox 模板规范（PART_EditableTextBox、PART_Popup）。2026-09-04 本机真实设置页 MiniMax 返回 8 个模型、火山返回 99 个模型，模型下拉选择已验收；仅请求目录，不发送用户提示词或附件。
- ProviderApiKeyEditorPolicy 将“显示已保存值”和“用户编辑”分开：前者只供原生 PasswordBox 掩码显示，不能形成待写凭据；后者明确创建替换/删除草稿。未经窗口防捕获确认不解密显示。2026-09-04 本轮 Release 全量 718 项通过，ProviderSmoke 编译零警告，Debug 界面已确认真实密钥逐字符圆点显示，验收不保存用户配置，GitHub 发布仍暂停。

# 请求体参数

- RequestParameters 是可选的旧配置兼容字段，所有编辑副本/凭据迁移克隆必须保留；显式 null 或非法字段在保存、Provider 构造与发送前拒绝，不得默默吞掉配置。ProviderRequestParameterPolicy 使用严格 JSON 对象解析，拒绝重复属性，仅允许 service_tier/temperature/top_p 及各自有限取值，禁止凭据和内部协议字段注入。
- 2026-09-04 官方 M3 最小实测：把 service_tier=priority 放 HTTP Header，响应仍为 standard；放请求体，HTTP 200 且 service_tier 明确回显 priority。两次均通过 MEWU_OK 挑战；短测不能证明整体加速比例。官方 OpenAI SDK 文档说明 priority 为标准价格的 1.5 倍，不得自动启用或因失败自动切收费档。
- 接入后 Release 全量 732 项通过，包含普通/流式请求体断言、持久化与旧配置兼容；Debug 设置页实际输入 priority JSON，关闭未保存。WPF 全局主题资源测试单独放入禁并行集合，STA 辅助线程为后台线程，避免与录屏等 WPF 测试竞争全局初始化或失败后挂住测试宿主。

- IncrementalMarkdownRenderer 按 Markdig 顶层块类型与原始跨度保留未变前缀，并在同一 FlowDocument 内替换变化后缀。旧/新正文含方括号时保守全部重建，以正确处理跨块引用、脚注和缩写；动作更新/字号改变也必须清理相应旧状态。新块生成后只替换实际 emoji Run，保留普通正文与代码布局。

- ApplicationUpdateService 的 confirmDownload 回调在官方版本与附件名称/URL 校验通过后、读取校验文件或写入下载目录之前执行；拒绝时仍返回 IsUpdateAvailable=true，但 Package=null。回调结束后再次检查取消，不能把发现更新等同于已下载。

- 更新器优先使用精确安装包资产的 `digest`，严格接受 `sha256:` + 64 位 ASCII 十六进制并归一为小写；不能使用 ZIP 等其他资产的哈希。digest 非空但非法时在询问/下载前失败，不得回退校验文件掩盖异常。只有字段缺失或 null 才允许旧校验文件，覆盖 GitHub REST 限流后官方 latest 重定向的兼容路径；无可信校验值则失败，不得无校验安装。
