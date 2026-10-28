# Mewu CLI / 命令行控制

The CLI introduced in MewuAI 1.0.3 lets other local agents operate the same Mewu desktop space as its user. Earlier 1.0.2 installers predate this interface. This is an inbound control interface, separate from Mewu's outbound MCP connections.

MewuAI 1.0.3 起提供命令行，让其他本机 Agent 与用户共同操作同一个 Mewu 空间。旧 1.0.2 安装包不包含此接口。它与 Mewu 调用外部工具的 MCP 连接是两个方向。

## Start / 开始

Start Mewu normally, then open a terminal and run:

先正常运行 Mewu，再打开终端：

```sh
mewu chat "你好"
mewu chat
mewu help
mewu status
mewu schema
mewu scene list --limit 20
```

### Terminal output / 终端输出

Current source opens interactive Agent chat when `mewu` is run in a terminal. `mewu --help` displays help; `mewu scene --help` and `mewu help call` provide command-specific help. With no terminal, bare `mewu` still prints offline help. Interactive status, scene, run and capability queries display readable text; piping or redirecting those queries preserves the original JSON response. `call` and `schema` remain JSON by default. Use `--output json` or `--output text` to select a format explicitly. The released 1.0.3 installers predate this chat interface and still use JSON help; these changes require a later build.

当前源码中，在终端直接运行 `mewu` 进入 Agent 对话，`mewu --help` 显示帮助；`mewu scene --help`、`mewu help call` 查看分级用法。没有终端时，裸 `mewu` 仍输出离线帮助。状态、会话、AI 任务和能力查询在终端使用文字排版，进入管道或文件时保留原 JSON。`call` 和 `schema` 默认仍为 JSON。可用 `--output json` 或 `--output text` 明确选择。已发布 1.0.3 没有此聊天入口且仍使用原 JSON 帮助，需要包含这些修改的后续构建。

```sh
mewu
mewu scene --help
mewu status --output json
mewu scene list --output text
```

### Chat with the Agent / 直接和 Agent 聊天

```sh
mewu chat
mewu chat "解释一下什么是 MCP"
mewu chat --scene 会话ID
mewu chat --agent "助理" --model "已有模型连接名称"
mewu chat "计算 6×7" --output json
```

`chat` starts a separate conversation on its first message. It sends text through the existing desktop `chat.send` operation, using Mewu's Agent engine, soul, memory, model and tools. It does not load another engine or access API credentials directly. By default it sends no screenshot/file references and will not overwrite an unsent draft in a resumed conversation. `--scene ID` explicitly selects a previous conversation. Replies remain in Mewu after the console exits.

`chat` 首次发送时创建独立会话，经原有 `chat.send` 使用 Mewu 的 Agent 引擎、灵魂、记忆、模型与工具，不另建引擎、不直接读取凭据。默认只发送文字，不附带截图或文件引用；续接的会话如果有未发送草稿，会拒绝覆盖。`--scene ID` 明确选择旧会话。退出控制台后，会话和答复仍保存在 Mewu。

Inside chat, use `/new`, `/agent [name or ID]`, `/model [name or ID]`, `/session`, `/use ID`, `/clear`, `/help` and `/exit`. Enter sends; the Up arrow recalls in-process input history. This input history is not written to a separate file. While an answer is running, Ctrl+C requests cancellation of the exact accepted run; a confirmed cancellation returns 130 in one-shot mode. At the input prompt, Ctrl+C exits the CLI with code 130. Ctrl+D or `/exit` exits normally. These actions leave the desktop application running. If the desktop switches scenes during a conversation, the next message is rejected rather than silently switching the user back; `/use ID` is an explicit switch.

对话内使用 `/new`、`/agent [名称或ID]`、`/model [名称或ID]`、`/session`、`/use ID`、`/clear`、`/help`、`/exit`。Enter 发送，↑ 浏览本进程的输入历史，不另存历史文件。回答中按 Ctrl+C 请求停止已确认的当前 run，单次提问确认停止后退出码为 130；等待输入时按 Ctrl+C 退出 CLI（退出码 130），Ctrl+D 或 `/exit` 正常退出。这些操作保留 Mewu 桌面程序运行。用户在桌面切换会话后，下一条消息会拒绝发送，使用 `/use ID` 明确切换，不会擅自抢回当前会话。

`chat --stdin` reads raw question text, whereas `call --stdin` reads a JSON params object. Chat defaults to answer text even when piped. `--output json` returns the final `{sceneId, runId, status, text}` response; `--jsonl` emits actual progress. The optional `run.get.stream` field contains a bounded in-memory live prefix for the exact scene/run, with an explicit `truncated` flag; stored final messages remain authoritative and long-term memory is unaffected. Older hosts without live-prefix support return their final message. `NO_COLOR` disables color.

`chat --stdin` 读取问题文字，`call --stdin` 才读取 JSON 参数对象。聊天即使进入管道也默认输出答复文字；`--output json` 返回最终 `{sceneId, runId, status, text}`，`--jsonl` 输出真实进度。可选 `run.get.stream` 仅提供对应 scene/run 的有界临时文字前缀，截断由 `truncated` 明示；最终以已保存消息为准，不限制长期记忆。旧宿主不支持前缀时仍返回最终答复。`NO_COLOR` 关闭颜色。

The official **命令行 / CLI** plugin is enabled by default. The control listener lives in the existing Rust desktop process; it does not start Node.js, Python, Electron or a second Agent engine. The console client does not open a second UI or independently open Mewu's databases.

官方 **命令行 / CLI** 插件默认开启。控制服务在现有 Rust 桌面进程内运行；命令行客户端连接该进程，不另开截图窗口或直接操作用户数据库。

On Windows, the installer adds its installation directory to the current user's PATH. Open a new terminal after installation. If Windows Terminal, an editor or another Agent still has the old environment, close and reopen that parent application; a new tab can inherit its old PATH. A portable build can run `.\mewu.exe status` from its extracted directory without changing PATH. Existing processes do not receive a replacement environment automatically. See [Windows environment inheritance](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables).

Windows 安装程序会将安装目录加入当前用户的 PATH，安装后需打开新终端。若 Windows Terminal、编辑器或其他 Agent 仍找不到命令，完全退出并重新打开该应用；仅新建标签页可能继承旧 PATH。便携版可在解压目录直接运行 `.\mewu.exe status`，无需修改 PATH。正在运行的进程不会自动换用新环境。

On macOS and Linux, starting an installed build with the CLI plugin enabled attempts user-level integration automatically: it creates `~/.local/bin/mewu` and configures a marked PATH block for a recognized Bash or Zsh startup file. Zsh uses `${ZDOTDIR:-$HOME}/.zshrc`; Linux Bash uses `~/.bashrc`; macOS Bash uses the first existing file among `.bash_profile`, `.bash_login` and `.profile`, or creates `.bash_profile`. Open a new shell afterward. macOS integration requires the app in `/Applications` or `~/Applications`; Linux integration points to the original AppImage in a persistent location, never its temporary mount. Restart Mewu from the new location after moving its app or AppImage.

macOS、Linux 的已安装构建正常启动且 CLI 插件开启后，会自动尝试用户级集成：创建 `~/.local/bin/mewu`，并为识别到的 Bash、Zsh 启动文件补充带标记的 PATH 区块。Zsh 使用 `${ZDOTDIR:-$HOME}/.zshrc`；Linux Bash 使用 `~/.bashrc`；macOS Bash 依次选择已有的 `.bash_profile`、`.bash_login`、`.profile`，均不存在时创建 `.bash_profile`。完成后打开新 shell。Mac 应先将软件放入 `/Applications` 或 `~/Applications`；Linux 的入口指向持久位置的原始 AppImage，不指向临时挂载目录。移动软件或 AppImage 后，从新位置重新启动 Mewu 以更新入口。

An unknown or missing `SHELL`, an unsupported shell such as Fish, or a protected/conflicting startup file requires manual PATH setup. The integrator preserves that file; creating a launcher does not prove that `mewu` is available in the current shell. Add `~/.local/bin` using your shell's own configuration, or invoke the launcher by its full path. For Bash/Zsh only, `export PATH="$HOME/.local/bin:$PATH"` updates the current shell; persist it in the startup file appropriate to that shell.

无法识别或未设置 `SHELL`、Fish 等未适配 shell，以及启动文件受保护或存在冲突时，需要手动配置 PATH；集成程序会保留原文件。创建了入口不代表当前终端已经能找到 `mewu`。请按所用 shell 的语法将 `~/.local/bin` 加入 PATH，或通过完整路径运行入口。仅对 Bash/Zsh，可用 `export PATH="$HOME/.local/bin:$PATH"` 临时设置当前 shell，持久设置写入相应启动文件。

Control requests from the Linux AppImage client, including `MewuAI.AppImage --cli status` and its generated launcher, require the `SO_PEERPIDFD` API provided by Linux 6.5+. Before sending request content it checks the actual socket peer's lifetime, the running executable bytes and the original AppImage identity. An unavailable API or a failed identity check rejects the connection without falling back to weaker verification. This requirement applies to the AppImage console connection, not to offline `help`/`version`, the screenshot UI or the Windows/macOS clients. The client must use the same original AppImage as the running desktop host. See [Linux 6.5 socket API definitions](https://github.com/torvalds/linux/blob/v6.5/include/uapi/asm-generic/socket.h).

Linux AppImage 客户端执行需要连接宿主的命令时（包括 `MewuAI.AppImage --cli status` 及自动创建的入口），需要 Linux 6.5+ 提供的 `SO_PEERPIDFD`。发送请求正文前会核验真实 socket 对端的进程生命周期、运行文件内容及原始 AppImage 身份；系统不提供该接口或身份校验失败时，直接拒绝连接，不降级验证。该要求针对 AppImage 命令行连接；离线 `help`/`version`、截图 UI 及 Windows、macOS 客户端不受此新增门槛影响。命令行与正在运行的桌面程序须来自同一个原始 AppImage。

Disabling or removing the plugin stops new console control without disabling normal screenshot UI. Already dispatched operations may have completed or may still be finishing; disabling a listener does not undo their effects. Query the operation or actual scene before issuing another action. The local connection authenticates its OS peer and verifies the expected desktop executable; it is not an unauthenticated HTTP server.

停用或卸载插件会停止新的控制台操作，截图 UI 仍可正常使用。已经派发的操作可能已完成或仍在收尾，关闭监听不等于撤销其结果；再次操作前应检查回执和实际场景。本地连接验证操作系统进程身份和桌面程序身份，不开放无认证 HTTP 控制端口。

## Discover the interface / 发现接口

[`apps/desktop/control/catalog.json`](../apps/desktop/control/catalog.json) is the versioned machine-readable catalog returned by `mewu schema`. Each entry includes its input schema, result shape, execution mode and example request. Resolve its schema references against the catalog's `$defs`.

`mewu schema` 返回[机器可读目录](../apps/desktop/control/catalog.json)，包含每个方法的输入 Schema、结果结构、执行方式和示例；其中的 `$ref` 使用目录顶层 `$defs`。只有目录中列出的固定方法可以调用，不接受任意 Tauri 命令名。

| Area / 范围 | Methods / 方法 |
| --- | --- |
| Status and discovery / 状态与发现 | `app.status`, `app.schema`, `app.capabilities`, `agent.list`, `model.list`, `plugin.list` |
| Scenes and results / 会话与结果 | `scene.list`, `scene.get`, `run.get`, `operation.get` |
| Capture / 截图 | `capture.open`, `region.add`, `image.export` |
| Conversation / 对话 | `chat.setDraft`, `chat.send`, `chat.cancel`, `references.set` |
| Blackboard and drawing / 黑板与绘制 | `blackboard.create`, `blackboard.open`, `blackboard.finish`, `drawing.apply` |
| Recording / 录屏 | `recording.start`, `recording.control`, `recording.status` |
| Scrolling capture / 长截图 | `scroll.start`, `scroll.control` |
| Files and capabilities / 文件与能力 | `asset.import`, `ocr.run`, `translation.run`, `pin.create` |
| Scene management / 会话管理 | `scene.new`, `scene.newConversation`, `scene.activate`, `scene.freeze`, `scene.close`, `scene.command`, `settings.open` |

The catalog describes supported entry points, not permission to bypass disabled plugins, absent model credentials or OS screen-capture dialogs. macOS permissions and Linux desktop portals still apply. A Wayland recording request may return `requiresSelection: true`; select a region on its newly authorized background before recording starts. Video trimming/export and any other UI feature absent from the catalog are not exposed by guessing native command names.

目录中的方法仍需通过真实插件状态、模型配置及系统截图授权。macOS 权限和 Linux 桌面 Portal 不会被跳过。Wayland 录屏可能返回 `requiresSelection: true`，需要在新授权的背景上选择区域后才开始录制。未列入目录的视频剪辑、导出等 UI 功能，不能通过猜测原生命令名调用。

## Requests and completion / 请求与完成

```sh
mewu call METHOD --json '{"argument":"value"}' --wait
mewu call METHOD --file request-params.json --wait
mewu call METHOD --stdin --wait
```

`--json`, `--file` and `--stdin` are mutually exclusive and contain the **params object**, not a complete request envelope. Files must be UTF-8 without a BOM. Use `--stdin` or `--file` when shell quoting would alter JSON. `call` outputs the original JSON response by default; `--output text` selects a readable view and does not change execution. `--json` selects input, not output. Diagnostics use stderr. `--jsonl` emits actual response progress as separate JSON lines and cannot be combined with `--output text`.

`--json`、`--file`、`--stdin` 三选一，内容是 **params 对象**，不是完整请求包。文件使用无 BOM 的 UTF-8；复杂文本建议用文件或标准输入，避免被 shell 改写引号。`call` 默认输出原协议 JSON，`--output text` 切换可读视图但不改变执行。`--json` 指定输入，与输出格式无关。诊断写入 stderr；`--jsonl` 按行输出真实响应进度，不能与 `--output text` 同时使用。

Operations first return an `operationId` with `status: "queued"`. **Queued is not completed.** `--wait` only polls `operation.get`; it never sends the original mutation again. `chat.send --wait` waits for the message to start a run, not for the entire AI answer. Read that run with `mewu run get SCENE_ID`.

写操作先返回 `operationId` 与 `status: "queued"`；**排队不等于完成**。`--wait` 只查询 `operation.get`，不会再次发送原操作。`chat.send --wait` 等待消息提交并启动回答，完整 AI 答复通过 `mewu run get SCENE_ID` 读取。

An operation's completed response retains the operation wrapper:

异步操作完成后仍保留操作回执外层：

```json
{
  "id": "poll-request-id",
  "result": {
    "operationId": "original-request-id",
    "status": "completed",
    "result": {
      "sceneId": "00000000-0000-4000-8000-000000000001",
      "revision": 42,
      "backgroundId": "00000000-0000-4000-8000-000000000002"
    }
  }
}
```

Check both the process exit code and the response. An operation failure can appear as `result.status: "failed"` with its own `result.error`. Error `outcome` distinguishes `not_sent`, `executed` and `unknown`; a lost reply is not proof that nothing happened. Neither the CLI nor host automatically retries mutations. `--timeout-ms` bounds a transport exchange; `--wait-timeout-ms` bounds polling.

应同时检查退出码和 JSON。业务失败可能出现在 `result.status: "failed"` 与 `result.error`，而非最外层 `error`。`outcome` 区分 `not_sent`、`executed`、`unknown`；丢失回包不代表未执行。CLI 和宿主均不自动重放写操作。`--timeout-ms` 控制单次通信期限，`--wait-timeout-ms` 控制轮询期限。

Supply `--request-id YOUR_UNIQUE_ID` when a controller needs a stable operation identity. An exact repeated request with the same ID uses the existing receipt while it remains cached; different parameters with that ID are rejected. Receipts are bounded, retained for at most ten minutes, and completed entries can be evicted earlier when the cache fills. Preserve the original `operationId` after a disconnect and query it rather than creating a new ID for the same action.

控制器可通过 `--request-id YOUR_UNIQUE_ID` 保存稳定身份。缓存内相同 ID、相同请求返回原操作回执；同 ID 不同参数会被拒绝。回执最多保留十分钟，缓存满时已完成记录可能提前淘汰。断开后应保留原 `operationId` 查询，不能直接换新 ID 重做同一动作。

The host rejects queued work after 30 seconds as `not_sent`. Once the renderer claims it, the completion wait is bounded to 300 seconds; a timeout becomes `unknown`. This deadline releases the control queue, not proof that the underlying native task was canceled. Inspect its actual scene or recording state before deciding what to do next.

宿主对排队超过 30 秒的操作返回 `not_sent`；界面接手执行后的回执等候最多 300 秒，超时返回 `unknown`。该期限用于释放控制队列，不代表底层原生任务已取消；下一步应根据实际场景或录屏状态决定，不自动重放。

`mewu call recording.status --json '{"id":"RECORDING_UUID"}'` reads the original recording’s terminal receipt: `active`, `finished`, `failed` (with a bounded native error), or `unknown`. Only the most recent completed recording is retained in memory; restarting Mewu or starting later work can make an older result unavailable. `finished` means the native task ended successfully, including a stop before media was produced: inspect `scene.get` for the resulting video before treating it as saved. `unknown` never authorizes replay. Calling without `id` preserves the current status-or-null response.

录屏停止后，可使用原录制 ID 查询结束结果和实际错误。`finished` 不替代成片检查，`unknown` 也不代表未执行；不要据此重新录制。无参数调用仍返回当前录屏或 `null`。

## Coordinates and shared state / 坐标与共同操作

Use real IDs and revisions from `app.status`, `scene.get` and `plugin.list`. `--scene ID --revision N` supplies optional `params.context` guards for a mutation. It fills missing context values without replacing explicit JSON values; native read-only queries do not accept context.

从 `app.status`、`scene.get` 和 `plugin.list` 获取真实 ID 与修订号。写操作可加 `--scene ID --revision N` 作为 `params.context` 前提；它只补充未填写值，不覆盖 JSON 中已有值。只读查询不接受该 context。

The space saves pending text, drawings, object placement, trim and geometry before a normal external mutation. An active gesture or conflicting UI action can return `BUSY`; changed targets return `CONFLICT`. A flush may legally advance the global revision, while native target IDs and exact drawing CAS remain authoritative.

普通外部写操作先通过原 UI 保存屏障，排空文字、笔迹、对象位置、剪辑和选区几何。用户正在拖动或执行其他操作时可能返回 `BUSY`；目标发生变化返回 `CONFLICT`。保存屏障本身可以提高全局修订号，最终仍由真实目标身份和原生绘制 CAS 验证。

Region geometry and ordinary drawing points use **original background pixels**. They are not normalized 0–1/1000 coordinates or CSS pixels. Drawing IDs must be UUIDs. `expectedRevision` is the current region's `drawingRevision`; obtain a new revision after each successful change. Rich layouts and tool provenance cannot be invented in a manual drawing request.

选区和普通笔迹使用 **原始背景像素坐标**，不是 0–1/1000 归一化坐标，也不是 CSS 像素。绘制对象 ID 使用 UUID；`expectedRevision` 使用该区域当前 `drawingRevision`，每次成功修改后重新读取。不得伪造富布局或工具来源。

## Capture to file / 截图导出示例

PowerShell 7 example. Adjust the region to the actual background size; use a new output path.

以下为 PowerShell 7 示例。框选范围需在实际背景尺寸内，导出路径应使用新文件：

```powershell
$capture = mewu call capture.open --wait | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Capture failed' }
$controlScene = $capture.result.result.sceneId
$state = (mewu scene get $controlScene --limit 1 | ConvertFrom-Json).result
$controlRegion = [guid]::NewGuid().ToString()
$params = @{
  backgroundId = $state.background.id
  region = @{ id = $controlRegion; x = 0; y = 0; width = 400; height = 240 }
} | ConvertTo-Json -Depth 8 -Compress
$params | mewu call region.add --scene $controlScene --revision $state.revision --stdin --wait

$output = Join-Path ([Environment]::GetFolderPath('Desktop')) 'mewu-capture.png'
@{ regionId = $controlRegion; variant = 'annotated'; path = $output } |
  ConvertTo-Json -Compress |
  mewu call image.export --scene $controlScene --stdin --wait
```

Use `variant: "original"` for the source image without annotations. `asset.import` accepts explicit paths, and its receipt contains new item/asset IDs that can be passed to `references.set` or `chat.send`.

`variant: "original"` 导出不带标注的原图。`asset.import` 接受明确路径，回执中的 item/asset ID 可继续用于 `references.set` 或 `chat.send`。

## Talk and draw / 对话与绘制

```powershell
@{
  text = 'Explain this screenshot'
  refs = @(@{ kind = 'region'; id = $controlRegion })
  regionId = $controlRegion
} | ConvertTo-Json -Depth 8 -Compress |
  mewu call chat.send --scene $controlScene --stdin --wait

mewu run get $controlScene --limit 5
```

`chat.send` uses the selected Agent engine, soul, model connection and memory provider. It does not bypass those settings. When replacing a draft or reference selection, supply `expectedDraft` or `expectedRefs` to reject a concurrent user edit.

`chat.send` 沿用当前 Agent 底座、灵魂设定、模型和记忆提供方，不绕过其配置。要覆盖草稿或引用选择时，可提交 `expectedDraft`、`expectedRefs` 防止覆盖用户并发修改。

The native host registers a command channel owned by the first-party space.
An accepted operation is still queued until that renderer flushes edits and
claims its exact nonce. Status observation may redeliver an unclaimed notification
at most twice, using the same operation identity; the renderer deduplicates it.
Claimed, completed, canceled, expired or reloaded operations are never dispatched
again. An unknown result remains unknown and must be inspected before any new
mutation is requested.

原生宿主通过第一方空间注册的命令通道交付请求。“已接受”仍表示排队，界面保存未提交编辑并认领精确 nonce 后才可执行。状态查询最多补送两次尚未认领的通知，始终使用原操作标识，由界面去重；已经认领、完成、取消、过期或随界面重载失效的操作不会重新派发。结果未知时仍需核对实际状态，不能当作未执行而重试写入。

For manual drawing, inspect the target region again and submit a complete `drawing.apply` command from the catalog example. The text drawing schema supports `bold`, `italic`, `underline` and `strikethrough`; a two-point text box wraps at its width. Undo and redo also require the current target revision.

手动绘制前重新读取目标区域，再按目录示例提交完整 `drawing.apply` 命令。文字支持加粗、斜体、下划线和删除线；两个点构成可自动换行的文字框。撤销和重做同样需要当前目标修订号。
