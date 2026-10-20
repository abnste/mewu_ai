# Mewu console 客户端

`mewu` 通过本机控制接口访问正在运行的 Mewu。独立 console 程序与桌面程序放在同一目录；客户端在发送正文前核验宿主进程身份。CLI 不自动启动 GUI，不读取聊天数据库、模型配置或 API Key。

```powershell
mewu
mewu chat "你好，帮我解释一下 MCP"
mewu chat --scene SCENE_ID
mewu help
mewu version
mewu status
mewu schema
mewu capabilities
mewu scene list --offset 0 --limit 20
mewu scene get SCENE_ID --limit 10
mewu run get SCENE_ID --limit 10
```

`help`、`version` 离线可用。其他命令需要 Mewu 已正常启动，且命令行插件已启用。`schema` / `capabilities` 返回当前宿主实际支持的方法；客户端不会假设未列出的方法可用。

当前源码中，在终端直接运行 `mewu` 或 `mewu chat` 进入连续对话；`mewu chat "问题"` 回答后退出。`mewu --help` 显示帮助，`mewu scene --help`、`mewu help call` 查看分级用法。`version` 输出普通版本文字。终端里的 `status`、`scene list/get`、`run get`、`capabilities` 使用文字视图，查询进入管道或重定向时仍输出原协议 JSON。可用 `--output text` 或 `--output json` 明确选择。已发布的 1.0.3 安装包尚无这些新聊天入口，需使用包含该修改的后续构建。

## 直接对话

默认在首次发送时新建独立会话，不附带截图、文件或当前草稿。`--scene ID` 继续已有会话；新消息使用原有 `chat.send`，沿用 Mewu 的 Agent 引擎、灵魂、记忆、模型与工具。CLI 不直接访问模型服务，也不单独保存 API Key 或聊天数据库。回答会保存在 Mewu；退出 CLI 不删除会话。

```powershell
mewu chat --agent "助理" --model "模型连接名称"
Get-Content -Raw -Encoding utf8 question.txt | mewu chat --stdin
mewu chat "计算 6×7" --output json
```

聊天中输入 `/help`、`/new`、`/agent`、`/model`、`/session`、`/use ID`、`/clear` 或 `/exit`。`/agent 名称或ID`、`/model 名称或ID` 选择现有配置，重名时使用完整 ID。Enter 发送，↑ 浏览当前进程的输入历史；历史不另写磁盘。回答中按 Ctrl+C 停止本次回答，等待输入时按 Ctrl+C 退出 CLI（退出码 130）；Ctrl+D 或 `/exit` 正常退出。停止请求绑定已确认的 run ID，不会取消另一个新回答，也不退出 Mewu 桌面软件。桌面主动切换会话后，不会在下一条消息时擅自切回；使用 `/use ID` 明确续接。

`chat` 默认输出答复文字，即使进入管道；`--output json` 返回最终 `{sceneId, runId, status, text}`，`--jsonl` 返回真实运行进度。`chat --stdin` 是问题文字，区别于 `call --stdin` 的 JSON 对象。正常完成退出码 0，已确认停止为 130；发送后断线或超时仍标记结果未知，不能重发原消息。新宿主通过 `run.get.stream` 提供有界、临时的文字前缀，最终已保存消息为准；旧宿主没有该字段时显示最终答复。`NO_COLOR` 关闭颜色。

终端对话用分隔线区分输入，用左侧对话线呈现回复，并按中英文显示宽度换行。粗体、标题、列表、行内代码和代码块使用现有 CommonMark 解析器显示；流式输出只更新未完成的行，历史内容留在终端滚动记录中。管道及 JSON 输出不加入这些装饰。

包含 CLI 的 Windows 安装程序会将安装目录加入当前用户 PATH。安装后打开新终端；若 Windows Terminal、编辑器或 Agent 已在运行，需重新打开该应用，不能保证仅新建标签页就会刷新环境。便携版可在解压目录直接运行 `.\mewu.exe status`。

macOS、Linux 的已安装构建在 CLI 插件开启后，正常启动会自动尝试创建 `~/.local/bin/mewu` 并为识别到的 Bash/Zsh 配置 PATH，完成后打开新 shell。Mac 需先安装到 `/Applications` 或 `~/Applications`；Linux 使用持久路径的原始 AppImage，移动后需从新位置重启 Mewu。未设置或未知的 `SHELL`、Fish 等未适配 shell、启动文件受保护或存在冲突时会保留原配置，需按自己的 shell 语法手动加入 `~/.local/bin`，也可用完整路径运行入口。创建入口不等于刷新当前终端环境。启动文件选择和完整示例见[CLI 使用说明](../../docs/cli.md)。

通用调用接受一个 JSON 对象，可选命令行、UTF-8 文件或 stdin 三种输入之一：

```powershell
mewu call agent.list
mewu call recording.status
mewu call chat.setDraft --json '{"text":"请检查当前截图"}' --scene SCENE_ID --revision 12 --wait
mewu call chat.send --file request.json --request-id agent-turn-001 --wait
Get-Content -LiteralPath request.json -Raw -Encoding utf8 | mewu call chat.send --stdin --wait --jsonl
mewu call operation.get --json '{"id":"agent-turn-001"}'
```

`--scene`、`--revision` 为 `params.context` 补充缺失的目标和一致性版本；已有字段保持原值。它们适用于接受 context 的宿主操作。先用 `status` / `scene get` 获取当前目标和版本；插件、场景或版本变化由宿主拒绝。方法专属参数（例如 `drawing.apply` 内的 `expectedRevision`）仍需按宿主 schema 提供。

`--request-id` 指定本次请求的幂等 ID，只允许 1–128 个 ASCII 字母、数字、`.`、`_`、`-`、`:`。未指定时生成 UUID。客户端从不自动重发原操作。宿主返回 `queued` / `accepted` 仅表示已接收；`--wait` 只轮询 `operation.get`，直到 `completed` / `failed` 或等待超时。同一 ID 的相同请求可由宿主在当前运行期内去重，不能据此承诺跨重启的永久去重。

`call`、`schema` 默认保留 JSON 输出，便于 Agent 和脚本使用；`--output json` 也返回原有完整 response（`id` 与 `result` 或 `error`）。`--jsonl` 每行输出一条实际响应，配合 `--wait` 可读进度，不能与 `--output text` 同时使用。`--wait` 的终态响应仍保留宿主的 `operationId`、`status` 与 `result` / `error`。诊断写入 stderr；文字模式不会将协议错误包混进 stdout。

| 选项 | 含义 |
| --- | --- |
| `--timeout-ms MS` | 单次连接或请求超时，默认 10000，最大 3600000 |
| `--wait-timeout-ms MS` | 总等待期限，默认 120000，最大 3600000，需同时使用 `--wait` |
| `--json JSON` / `--file PATH` / `--stdin` | 通用 `call` 的对象参数，三选一，默认 `{}` |
| `--output text\|json` | 选择输出格式；与输入参数 `--json JSON` 无关 |
| `--jsonl` | 每行一个响应；不改变执行次数 |

JSON 输入和最终请求均最多 1 MiB。重复 flag、未知参数、重复 JSON 字段、无效数字和非对象参数会在发送前拒绝。文件必须是 UTF-8 JSON，不能含 BOM。

| 退出码 | 含义 |
| --- | --- |
| 0 | 查询成功，或已取得接收/完成响应；未使用 `--wait` 时不能据此认定操作已完成 |
| 1 | 宿主拒绝或操作明确失败 |
| 2 | 输入格式或大小无效 |
| 3 | 宿主不可用或已断开 |
| 4 | 本机身份或权限校验失败 |
| 5 | 已知未执行的请求超时 |
| 6 | 原操作执行结果未知；使用响应中的 operation ID 查询，勿自动重复 mutation |
| 74 | stdout 无法写入 |

桌面程序的 `--cli` 入口复用同一解析和调度模块，在创建 WebView 前分流；例如 Linux AppImage 可用 `MewuAI.AppImage --cli status`。AppImage 专用入口及自动创建的 launcher 在连接宿主时要求 Linux 6.5+ 提供的 `SO_PEERPIDFD`，以绑定真实 socket 对端的完整进程生命周期，并在发送正文前验证双方运行文件的完整 SHA256 和同一原始 AppImage 文件身份。API 不可用或身份校验失败会拒绝连接，不自动回退到较弱验证；客户端与桌面程序须来自同一个原始 AppImage。该门槛针对 AppImage 命令行连接，离线 `help`/`version` 与截图 UI 不受此新增要求影响；普通客户端仍验证宿主完整路径，Windows、macOS 验证不变。

`tests.rs` 覆盖真实协议 Request / Response 的解析与本地 mock 调度：输入限额、重复 JSON、context 保留、操作终态/未知结果、stdout 失败和原操作不重发。真实本机传输、插件准入和 GUI 操作需另做集成验证。
