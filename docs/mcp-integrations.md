# MCP 设置与内置集成 / MCP settings and built-in integrations

## 中文

在 **设置 → MCP** 配置邮箱、截图分享及笔记服务。这个栏目中的连接方式各有不同：

| 服务 | 连接方式 | 用途 |
| --- | --- | --- |
| QQ 邮箱 | OAuth 扫码授权 + MCP | 邮件相关提问读取邮箱上下文；确认草稿后请求发件 |
| 网易邮箱 | 扫码网页会话；SMTP 授权码 | 扫码用于读取收件箱，SMTP 授权码用于发件，两者分别验证 |
| 钉钉 | 企业内部应用 OpenAPI | 将当前截图作为工作通知提交给指定 userid |
| 飞书 | 企业自建应用 OpenAPI | 将当前截图发给群聊 chat_id 或联系人 open_id |
| ima | Client ID / API Key + OpenAPI | 将当前截图归档到可写知识库 |
| Obsidian | 本地 vault 文件 | 保存 PNG 和引用图片的 Markdown 笔记 |

### 配置与使用

1. 填写所需账号或应用信息，通过页面的保存按钮保存 Secret、API Key 或 SMTP 授权码；凭据使用 Windows DPAPI 在本机加密。
2. 点击“测试连接”。QQ 邮箱会验证账号；网易分别显示收件箱会话及 SMTP 验证结果；飞书加载可见群列表；ima 加载可写知识库。测试不会发送邮件或分享截图。
3. 选择分享目标，开启服务并保存设置。飞书选择群聊时会同时切换到 `chat_id`；ima 默认选项表示第一个可写知识库，也可指定一个知识库。已指定的知识库不可用时会保留选择并提示重新选择。
4. 下次圈选静态截图后，点击对应分享按钮。Obsidian 保存到本地；钉钉、飞书及 ima 会上传当前选区的图片。视频区域不显示这些图片分享入口。

邮箱地址的后缀不决定发件账号；同时配置两个邮箱时，可在撰写入口选择本次账号。应用在提交 QQ/网易发件请求前展示草稿供确认，失败后不会自动换账号重发。若返回结果无法确认投递，请先检查“已发送”文件夹，避免重复发送。钉钉的工作通知是异步任务，提交成功表示平台接受任务。

QQ 邮箱以 MewuAI 的客户端名称申请授权。是否接受新客户端由邮箱平台决定；平台拒绝注册时会显示相应错误。网易扫码依赖其网页会话，过期后需要重新扫码；发件需在邮箱设置中启用 SMTP 并生成授权码。

Obsidian 的附件和笔记目录使用 vault 内相对路径；可以将笔记目录留空，保存到 vault 根目录。保存失败或取消时清理本次未完成文件，已有笔记保留。为保证保存位置明确，目录不能经过符号链接或目录联接。

截图和标注按钮的小字提示可在 **设置 → 常规 → 显示按钮功能文字** 开关。文字放在按钮内部，开启时不会增加工具栏高度；保存后下次截图生效。

## English

Use **Settings → MCP** to configure built-in mail, screenshot sharing and note integrations.

| Service | Connection | Purpose |
| --- | --- | --- |
| QQ Mail | OAuth QR authorization and MCP | Read context for mail-related questions; request sending after draft confirmation |
| NetEase Mail | QR web session; SMTP authorization code | Read the inbox through the web session; send through SMTP, with separate checks |
| DingTalk | Internal enterprise app OpenAPI | Submit a screenshot as a work notification to recipient userids |
| Feishu | Enterprise app OpenAPI | Send a screenshot to a chat_id or open_id |
| ima | Client ID / API Key and OpenAPI | Archive a screenshot in an addable knowledge base |
| Obsidian | Local vault files | Save a PNG and a Markdown note referencing it |

Enter the account or application details and use the page's save button to store its secret or authorization code. Credentials are encrypted locally with Windows DPAPI. **Test connection** checks the configured account, loads Feishu chats or loads ima knowledge bases without sending mail or sharing screenshots. Enable the service and save Settings, then use its button on a static capture.

Selecting a Feishu chat also selects `chat_id`. In ima, **Default** means the first addable knowledge base; an explicit saved choice remains selected if it becomes unavailable, so it can be corrected without silently choosing a different library.

Recipient domains do not choose the sending account. When both mail accounts are available, the compose action offers an account choice. Drafts require confirmation before sending; failed requests are not automatically retried through another account. If delivery cannot be confirmed, check Sent before resending. DingTalk notifications are asynchronous: acceptance means the platform queued the task.

QQ Mail authorization uses the MewuAI client name and depends on the platform accepting that registration. NetEase QR sessions may expire and require another scan; SMTP sending requires an authorization code and enabled SMTP access.

Obsidian folders are relative to the vault; an empty notes folder means the vault root. Failed or canceled saves clean up files created by that operation. Symbolic links and junctions are rejected to keep the destination inside the selected vault.

**Settings → General → Show button labels** controls compact text inside capture and drawing buttons. It does not increase toolbar height and applies to the next capture after saving.

## 官方参考 / Official references

- [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle) and [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)
- [QQ Mail API](https://api.mail.qq.com/)
- [DingTalk open platform](https://open.dingtalk.com/)
- [Feishu image upload](https://open.feishu.cn/document/server-docs/im-v1/image/create) and [message sending](https://open.feishu.cn/document/server-docs/im-v1/message/create)
- [ima agent interface](https://ima.qq.com/agent-interface)
- [Obsidian URI](https://help.obsidian.md/Extending+Obsidian/Obsidian+URI)
