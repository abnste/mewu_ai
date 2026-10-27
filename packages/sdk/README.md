# @mewu/sdk

Mewu API 插件运行在独立 QuickJS worker 中。宿主提供 SDK 模块，安装插件不需要 Node.js；此 npm 包用于开发时的类型提示与契约测试。

仓库中的 0.2.0 是开发版本，新增通用客户端与对象事务。下面的已发布 0.1.0
下载仍可用于既有插件，但不包含这些新增类型；开发新接口应使用本仓库对应 SDK。

在独立 ESM 项目中安装已发布的开发 SDK：

```sh
npm init -y
npm pkg set type=module
npm install --save-dev --ignore-scripts --no-audit --no-fund https://github.com/abnste/mewu-plugins/releases/download/v0.2.2/mewu-sdk-0.2.2.tgz
```

运行时 SDK 由 Mewu 提供；不要把 `node_modules` 放入插件包。打包、自己的 GitHub Release 和社区 PR 流程见 [作者指南](https://github.com/abnste/mewu_ai/blob/master/tools/extension-market/README.md#from-an-independent-project-to-a-market-pr)。

```js
import { definePlugin } from '@mewu/sdk';

export default definePlugin(ctx => {
  ctx.commands.register({ id: 'example.hello', title: '你好',
    metadata: { placement: 'space-toolbar', input: { name: 'Mewu' } }
  }, async input => {
    return { content: `你好，${input.name || 'Mewu'}` };
  });
});
```

插件目录只包含 `mewu-plugin.json` 与 `.js` / `.mjs` 模块。清单需声明 `schemaVersion: 2`、`hostApi: 2`、`runtime: "javascript"` 和入口文件。完整示例在仓库 `extensions/examples`。

清单可声明自己的 `icon`，市场、已安装列表与截图窗口的插件按钮共用它：

```json
"icon": {
  "paths": ["M4 4h16v16H4z", "M8 8l8 8"],
  "color": "#8b5cf6",
  "filled": false
}
```

路径使用固定 `0 0 24 24` SVG 坐标系，最多 16 条、每条 4096 字节、合计 8192 字节；颜色可省略以适配当前主题，或指定六位十六进制颜色。`filled` 默认 `false`。只接受路径数据，不接受 SVG/HTML 字符串、脚本、外部图片或 URL。未声明时按插件功能显示默认图标。此可选字段需使用支持图标的新版宿主；旧插件包无需重发，市场条目可提供图标，但不能覆盖包内已声明的不同图标。

普通 JavaScript 插件省略清单 `platforms`、市场条目填写 `platforms: ["any"]`，同一个 ZIP 即可在 Windows、macOS、Linux 使用同一套 SDK。仅整包确实依赖本机能力时，才填写 `platforms: ["windows-x86_64"]` 等允许清单；另两个目标为 `darwin-aarch64`、`linux-x86_64`。市场不得声明比包更广的平台，`any` 不能与具体平台混用。本地安装、启用、回退、依赖和运行也受宿主校验，无法仅绕过按钮安装。

同包只有个别功能受限时，用服务 `metadata.platforms` 限制该功能，其他 UI、记忆等继续跨平台。`metadata.requiresRegion: true` 会核对宿主原生区域后端及系统授权，不支持的桌面会显示禁用原因；不能仅凭插件是 JavaScript 就宣称所有本机能力都已实现。处理函数内可按宿主提供的 `context.platform`（`windows` / `macos` / `linux`）适配可选行为。

可注册命令、工具、Agent、记忆、模型、通讯渠道、渲染器和 UI 服务。注册发生在激活阶段；返回的函数可注销服务，`ctx.onDispose` 可登记卸载清理。服务的设置、菜单位置等通过 descriptor 的 `metadata` 声明。API 类型以 `index.d.ts` 为准。

### 设置插槽和本机 Agent 协议（开发版本）

`ctx.runtime.describe()` 返回当前宿主的能力名称。依赖新接口的插件应在激活时检查，不能将相同 API 格式版本当成所有可选能力都存在。

UI 服务声明 `metadata: {surface: 'settings'}`，返回 `{html,width,height}`，即可在设置侧栏注册入口。客户端仍通过隔离 MessagePort 的 `mewu.call()` 调用自己的服务；没有 Tauri、任意 IPC 或其他插件的接口。关闭页面、停用和更新撤销旧文档。

声明 `configuration.read` 后可用 `ctx.configuration.get(key)` 读取插件共享配置。写入需要 `configuration.write` 且调用来自该插件的活动设置文档，使用 `compareAndSet(key,expected,value)` 防止覆盖并发更改。普通 `ctx.storage` 继续按 Agent 作用域隔离，用于各角色的会话和记忆。

声明 `process.rpc` 后，`ctx.processes.resolve(candidates,files)` 可检测有界的安装路径；`${home}`、`${appData}`、`${localAppData}`、`${programFiles}` 是宿主展开的系统路径。`open({executable,args,cwd?,env?})` 启动固定本机可执行文件并返回 `{id,cwd}`。参数为数组，禁止 shell/自动安装器。`write(id,message)` 和 `read(id,timeoutMs)` 交换 JSON 行；读取超时返回 null，关闭/坏 JSON/资源超限返回失败。协议、请求 ID、重试和会话规则由插件实现。

本机程序按正常用户环境运行，拥有该用户权限；它不是 QuickJS 沙箱的一部分。运行包必须如实说明这一权限。进程句柄仅在本次调用有效，`close`、完成、取消、插件停用或卸载会清理它；不提供永久后台进程或跨插件句柄。

若声明 `media.read`，`write` 可附加 `{frameId,pointer,encoding:'base64'|'dataUrl'}` 绑定。宿主只把当前 Agent 输入中已授权的图片注入消息的 null JSON Pointer 占位，不向插件开放任意文件路径或他人素材。这个接口与所有平台协议无关。示例：[外部 Agent 插件](../../extensions/official/agent-connectors)。

让普通 Agent 按任务调用的能力使用 `ctx.tools.register`，提供明确的 `description` 和 `inputSchema`。需要本次用户授权选区的工具声明 `metadata: { requiresRegion: true }`；只有已明确授权区域的运行才会向模型提供该工具，插件也应拒绝缺少 `context.regionId` 的调用。注册工具不会创建截图工具条命令，也不会自行获得区域授权。

提供方专属工具声明 `metadata.requiresProvider: 'memory' | 'agent'`，仅当前 Agent 选中的同包、同类型服务可提供；执行期间切换提供方会撤销原授权。记忆工具可另设 `requiresMemory: true`；选择记忆提供方本身也要求长期记忆开启。宿主自动同步已完成对话时，Memory 服务收到只读 `context.memoryOrigin: 'completedTurn'`，普通工具参数不能获得这个来源标记。插件自行决定是否启用自动同步，不应把所有 retain 请求都当作用户明确要求保存。

[`region-agent`](https://github.com/abnste/mewu-plugin-region-agent) 的 0.1.2 版将区域操作注册为工具，并保留独立 Agent 服务；Agent 在没有选区时仍可正常聊天。它继续使用 API 与 SDK 0.1.0 已有的注册接口，普通 Agent 的自动工具调度需要支持该能力的新版宿主。独立仓库附有 Windows 原生运行截图。更新插件时发布新的版本与 ZIP，不覆盖社区已发布的资产。

宿主按插件和 Agent 隔离持久存储。每条记忆可以独立保存，通过分页键目录检索；不存在固定 64 条的模型记忆槽位。`storage.list` 的 cursor 用于分页，prefix 用于过滤，不能互相替代。并发读改写使用 `storage.compareAndSet(key, expected, value)`：返回是否成功，`expected: null` 明确表示键不存在，不匹配已保存的 JSON null。单值及一次请求有资源限制，长期数据应拆分为独立记录。

模型调用使用宿主已配置的连接，插件不获得 API Key。区域观察返回宿主图片 artifact ID；将该 ID 放到 `ctx.model.invoke({ prompt, images: [artifactId] })`，由宿主提供真实图片。区域操作还需要用户此次明确发起的运行授权，仅有清单权限不够。坐标是观察图内的像素，不能传桌面全局坐标。

宿主调用失败会拒绝 Promise。新版 SDK 的 `Error.code` 提供有界的公开错误码，例如
`region_window_boundary`、`region_target_changed` 或 `MODEL_REQUEST_FAILED`；不提供原始
网络响应、密钥或堆栈。旧宿主或未知错误可只有 `HOST_ERROR`。插件可将观察/模型失败
转成 `{isError:true, completed:false, content:"可操作的原因"}`，交回调用它的 Agent。
输入调用失败可能已经产生效果，不能据错误码自动重放；输入成功但反馈失败时应保留
已执行步数并注明需要重新观察，不能声称整个任务未执行。

### 插件界面与对象事务（开发中，需要新版宿主）

插件 UI 自己返回 HTML/CSS/JavaScript。以 `ctx.ui.register` 注册
`metadata: {surface: "space-overlay"}`，处理 `{type:"mount"}` 并返回
`{html, width, height}`。宿主可同时显示多个独立服务，不认识九宫格、文件夹、
垃圾桶或某个插件 ID。声明 `ui.render` 权限；对象持久化再声明 `objects.library`。
增加 `presentation: "object"` 可使用通用浮动对象外壳：顶部拖动条、大小调整、
置顶与关闭由宿主处理，内容继续由插件提供。置顶对象可在截图空间关闭后继续编辑；
应用退出后重新打开插件恢复其持久内容。停用或卸载仍会立即撤销对象的权限。

客户端文档是持续存在的隔离沙箱。内嵌客户端 API：

- `await mewu.ready` 等待当前文档的专属 MessagePort。
- `await mewu.call(input)` 调用自己的已注册服务。不能选择其他插件、IPC 或文件路径。
- `mewu.on(name, callback)` 接收 `drag` 与 `change` 事件，返回解除监听函数。
- `mewu.onBeforeClose(async () => save())` 在 Esc 或关闭前保存编辑内容；
  回调失败保留界面和正文并报告错误，不能把失败视为已保存。
- `mewu.setDropTargets([{id,x,y,width,height,default?}])` 声明文档内的放置区域。至多一个区域可标记 `default: true`，用于用户直接把对象放到此插件入口；界面首次加载时，宿主会等待接收区域就绪。默认位置及收藏规则由插件决定。
- `await mewu.viewportPoint(x,y)` 将客户端坐标转换为当前空间比例坐标。
- `await mewu.drag(pointerDownEvent)` 接续一次真实左键手势，返回
  `{cancelled,x,y}`（客户端坐标）。宿主只保证跨文档收到松手；移动、重排、删除、
  恢复对象的决定由插件做。Esc、关闭、失焦和 30 秒超时取消手势，不能重放旧事件。
  浏览器端类型可引用 `@mewu/sdk/client`。
  可传第二个参数 `{image, label}` 展示跟随鼠标的拖动预览；图片仅接受受限大小的 PNG data URL。

处理拖入时输入为 `{type:"drop",target,kind}`。来源引用由第一方界面单独传递，
插件 JS 不能自行构造。`ctx.objects.commit({expectedRevision,state,captureKey,remove})`
在一个事务内保存任意 JSON 状态并保留/移除对象；`captureKey` 消费当前实际拖放
来源一次，失败不会自动重试该手势。`ctx.objects.get()` 返回 `{revision,state,objects}`，
目录含名称、类型、尺寸，没有本机路径和原会话。`ctx.objects.restore(revision,key,{x,y})`
取出新副本。插件返回 `{removeSource:true}` 时，只有仍有效的当前用户拖放动作才能
移除原对象，普通 RPC 不能借此删除场景内容。
`ctx.objects.preview(revision,key)` 返回 `{image}`，包含不超过 240×180 的图片对象
PNG 缩略图（含标注）。HTML/SVG 对象返回 `{image:null, document:{html,width,height}}`；界面可调用 `await mewu.previewDocument(result.document)` 生成静态缩略图（优先 PNG，浏览器不支持栅格化时返回 SVG）。该方法使用一次性的隔离文档，无网络和宿主 API，结束、超时或界面关闭即销毁。不支持预览的类型返回 `image:null`，不暴露本机文件路径。

关闭文档、切换会话、插件停用/卸载或版本变化，会撤销旧界面的操作资格；持久数据在
卸载后保留。无布局和目录层级规定。单插件上限：256 KiB JSON 状态、4096 个对象、
32 MiB 模板；客户端 HTML 256 KiB。首次激活用 `ctx.objects.describe()` 检查
`capabilities` 是否包含 `ui.surfaces`，旧版宿主应明确拒绝，不注册不可用按钮。
需要浮动对象或缩略图的插件还应检查 `ui.objects`、`objects.preview`。

完整示例：`extensions/official/object-toolbox`（九宫格、文件夹、拖出、垃圾桶）和
`extensions/examples/research-notes`（编辑便笺、列表引用）。两者只通过同一通用接口，
均可独立打包。它们尚未发布到市场，不能把开发源码说成已安装的公开版本。

worker 当前按服务调用创建；通用设置插槽见上文，常驻后台订阅与第三方场景对象 codec
仍未完成。具体边界见 `docs/plugin-boundaries.md`。不要把注册方法存在当作产品接线完成。

区域自动操作的原生适配器支持 Windows、macOS 和 GNOME 50 Wayland x64。macOS 需要辅助功能和录屏权限；GNOME 需要 Mewu Desktop Witness 与 Portal 当次授权。其他 Linux 桌面尚未适配区域输入。区域须来自真实截图、位于同一个前台应用内容区，目标身份、显示器、选区与输入焦点会持续校验；密码框不允许填写，Esc 或宿主停止可撤权。输入送达不等于应用完成更新，插件应重新观察反馈；独立区域示例允许 `{observe:true}` 获取新图而不重放输入，观察也占用步骤预算。

Windows 的 layered 窗口同时具有 `WS_EX_TRANSPARENT` 时，鼠标输入由系统穿透到下方应用，这类叠层不会阻止区域操作，但仍可能出现在授权区域的截图中。宿主不承诺截图只包含目标应用的像素；观察和鼠标操作仍校验实际命中目标，检查不能锁住外部窗口的后续变化。

运行时没有 `process`、`require`、任意文件访问或直接网络接口。Node.js 的包不是可直接移植的宿主库；需要使用 SDK 服务或在插件包内提供兼容的 ESM 模块。

发布包的 `index.mjs` 是 `crates/mewu-extension/src/sdk.js` 的精确副本。修改嵌入式源后运行 `npm run sync`，`npm run check:sync` 与 `prepack` 拒绝两个版本分叉；npm 包不引用仓库外路径。

### 历史记忆迁移

宿主能力 `memory.archive` 提供旧记忆账本的分页读取和受控凭据移交，服务协议与迁移规则由独立插件实现。插件声明 `memory.archive.read` / `memory.archive.write`，调用 `ctx.memoryArchive`；凭据移交另需 `secrets.write`。

设置中的密码框可通过 `secrets.settings` 能力及单独的 `secrets.read` 权限，调用 `ctx.secrets.read(key, expectedRevision)` 回填此插件已保存的真实值。它只允许本插件、当前版本、仍有效的设置界面调用；Agent、工具、空间对象和其他插件不能读取。密码使用 `input type="password"` 显示，离开界面时清空；不得放入插件存储、日志或模型上下文。不需要回填时继续使用 `secrets.status` 与网络授权句柄。

`list`、`page`、`credential`、`activate` 仅允许当前存活的设置界面调用。`credential` 将指定历史连接的凭据复制到当前插件的 `archive.<bindingId>` 托管名称，固定为原服务 origin，不返回明文。`activate` 校验历史连接版本、当前扩展绑定版本和本插件注册的记忆服务后才切换提供者。原账本、凭据与待处理任务保留，迁移读取本身不发送网络请求。

运行期 `lookup` 和 `forget` 仅允许当前记忆调用的 Agent 访问自己的历史连接；返回提交状态和遗忘记录，插件不得重发不确定写入或恢复墓碑。插件停用、卸载、绑定变化或设置窗口关闭都会撤销这些调用的资格。


### 对话条的 Agent 模型目录（Mewu 1.0.10+）

Agent 注册描述可声明 `metadata: {modelsService: "models"}`。该 ID 必须指向同一插件注册的普通 `service`，不能引用其他插件。宿主按注册信息呈现引擎和模型菜单，不识别厂商名称。

目录服务接收 `{type:"list"}`，返回 `{ok:true, models:[{id,name}], selectedModelId, selection}`。`selection` 是该会话的当前选择状态，首次可为 null；宿主原样传入 `{type:"select", modelId, expected:selection}`。空 `modelId` 表示默认。插件校验实时可用模型，用 `ctx.storage.compareAndSet` 保存，并返回同形状的提交结果；失败返回 `{ok:false,error}`。`context.agentId`、`context.sceneId` 由宿主提供，模型选择不能串入其他角色或会话。Agent 执行时应读取同一选择。最多 512 个模型，ID 和名称不超过 300 字符。

宿主在发送期间禁止切换，切换会话、卸载、停用、升级后丢弃迟到目录；认证、模型探测及保存均由插件完成。仅注册目录不表示账号有权限或余额，首次实际请求仍可能失败。

`process.installation-search` 能力允许 `ctx.processes.resolve(candidates, files, searches)` 的第三个可选参数：`[{root:"${localAppData}/Vendor/App/bin", relativePath:"client.exe"}]`。只检查明确根目录的一级版本子目录，不递归扫描磁盘、不解释 shell 或通配符；按修改时间从新到旧检测，每个根最多读取 256 项、选取 8 个版本，最多 8 个根。路径规则属于插件；宿主不包含厂商目录。旧宿主未声明该能力时不得发送 `searches`。手动配置必须保留，不因自动发现结果而被覆盖。

### 工具条布局服务（Mewu 1.0.11+）

通过 `toolbar-layout` 服务重排原有截图工具条按钮，设置与顺序由插件保存；停用后恢复默认顺序，不替换原有按钮事件。完整输入、返回值及稳定按钮 ID 见[布局协议](../../docs/plugin-toolbar-layout.md)。内置 Agent 的技能和插件工程工作流见[制作协议](../../docs/agent-skills.md)。
