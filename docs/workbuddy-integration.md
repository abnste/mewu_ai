# WorkBuddy 本机 Agent 接入

2026-09-06，基于 `master` 的 `b1ccc78` 实现。当前验证的官方客户端版本为 WorkBuddy 5.5.3；应用版本保持 0.2.6，本次不发布新标签。

## 接入方式

通过本机 WorkBuddy 自带 CLI 的 ACP stdio 接口运行独立会话。使用安装目录中的 WorkBuddy.exe 与 bundled CLI，复用官方登录目录，由官方运行时管理认证与额度。喵呜AI不复制认证信息，不调用会触发重新登录的 authenticate，也不调用包含访问令牌的 getUserInfo。

设置页自动读取真实模型目录及思考选项；这一步不发送推理消息。0.3.0 的手动测试连接改为 ACP 握手、会话和配置协商，独立 30 秒超时，不发送推理消息；此结果只验证协议连接。早期 MEWU_OK 挑战是下方历史验收记录，不再是当前设置页的测试方式。WorkBuddy、API、Hermes、Codex 可以同时保留配置，覆盖层会在存在多个可用渠道时按本轮选择；各自配置和历史按渠道隔离。

## 图片与视频

图片使用 ACP image 内容块。MP4 使用独立工作目录中的文件副本交接，附带完整附件顺序的 regionIndex，由 WorkBuddy 自己调用本机工具分析画面和时间轴。不会因为 ACP 缺少原生 video 内容块就关闭 Agent 的视频能力，也不由喵呜AI预先抽帧替代 Agent。

视频请求在后台启动时启用限定本机工具，并验证 sandbox 已启用；其他请求关闭本机工具。每轮关闭用户/项目设置来源、hooks、插件、MCP 与记忆继承，不改 WorkBuddy 全局配置。额外权限请求拒绝，输入副本与派生文件统一通过临时租约回收；请求取消或结束即终止自己的后台进程树。

不下载或捆绑视频工具。不同电脑已有的解码工具会影响 Agent 的处理方法、速度及成功率，不能用一次本机验收保证所有视频都能识别。模型也可能识别错误；应明确报告无法实际检查的附件。

## 完成状态与隐私

只接受匹配当前会话的消息；忽略子 Agent 和内部压缩消息，拒绝结束后的迟到消息。工具调用前的临时说明不写入最终答案和历史。必须收到 end_turn 与非空最终正文；取消、截断、仅思考、空正文以及终端错误均失败。

官方 5.5.3 的 resolveTaskOutcome / determineTaskOutcome 会在任何工具调用曾报错时把已完成回答记为 PARTIAL_SUCCESS，即使之后换工具成功。因此该统计值并不等于协议截断；仅在 end_turn、无终端 errorMessage 且有最终正文时接受。其他失败统计值拒绝。模型返回的内容仍按统一结构化回答和视频批注规则校验。

请求最多 16 个附件，单图 20 MiB、单视频 512 MiB；图文内联请求有 64 MiB 上限，视频为本机文件交接。历史沿用统一 20 条、24,000 字符边界。拥有的截图字节和临时读写缓冲在失败、取消和完成路径均清零，不记录后台原始输出。

## 本机验收

- 官方模型目录返回 15 项，模型及思考选项设置成功。
- 文字挑战准确返回 MEWU_OK；256×256 合成红图准确返回 MEWU_RED，截图缓冲已清零。
- 4 秒合成 MP4：Agent 自行发现本机已有解码工具、检查原视频，识别开头红色、结尾蓝色与约 2 秒的切换时间；发送前后源文件 SHA-256 一致。
- 视频实测中 Agent 使用了本机 FormatFactory 中已有的 ffmpeg.exe；该文件没有进入喵呜AI依赖、源码或发布包。
- 843 项 Release 测试通过，1 项需显式启用的真实音频设备验收跳过；主程序、测试与 ProviderSmoke 构建零警告。
- 四个设置页在三种窗口宽度和 100%/125%/150%/200% 渲染比例下检查边界及对齐，主页使用合成 WorkBuddy 配置验证。没有保存或上传用户真实设置。

本次没有重新执行区域录屏到覆盖层视频批注的完整发布验收，也没有进行其他电脑实测；正式发布时仍须遵守仓库的完整录屏与视频批注验收流程。

## 官方依据

- [CodeBuddy ACP 接口](https://www.codebuddy.cn/docs/cli/acp)
- [CLI 命令参考](https://www.codebuddy.cn/docs/cli/cli-reference)
- [SDK 接入与配置来源](https://www.codebuddy.cn/docs/cli/sdk)
- [ACP Prompt turn](https://agentclientprotocol.com/protocol/v1/prompt-turn)
- [ACP 会话配置](https://agentclientprotocol.com/protocol/v1/session-config-options)

桌面客户端的捆绑路径及 outcome 统计语义同时核对了本机官方 5.5.3 安装产物。腾讯未来升级可能改变安装布局或接口；找不到入口、目录无效、模型消失或沙箱未生效时应明确失败，不静默改用其他账户或 API。
