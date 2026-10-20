# @mewu/sdk

Mewu API 插件运行在独立 QuickJS worker 中。宿主提供 SDK 模块，安装插件不需要 Node.js；此 npm 包用于开发时的类型提示与契约测试。

仓库中的 0.2.0 是开发版本，新增通用客户端与对象事务。下面的已发布 0.1.0
下载仍可用于既有插件，但不包含这些新增类型；开发新接口应使用本仓库对应 SDK。

在独立 ESM 项目中安装已发布的开发 SDK：

```sh
npm init -y
npm pkg set type=module
npm install --save-dev --ignore-scripts --no-audit --no-fund https://github.com/abnste/mewu-plugins/releases/download/v0.1.0/mewu-sdk-0.1.0.tgz
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

同包只有个别功能受限时，用服务 `metadata.platforms` 限制该功能，其他 UI、记忆等继续跨平台。`metadata.requiresRegion: true` 的原生区域操作目前仅 Windows 可用，其他系统显示禁用原因；不能仅凭插件是 JavaScript 就宣称所有本机能力都已实现。处理函数内可按宿主提供的 `context.platform`（`windows` / `macos` / `linux`）适配可选行为。

可注册命令、工具、Agent、记忆、模型、通讯渠道、渲染器和 UI 服务。注册发生在激活阶段；返回的函数可注销服务，`ctx.onDispose` 可登记卸载清理。服务的设置、菜单位置等通过 descriptor 的 `metadata` 声明。API 类型以 `index.d.ts` 为准。

### 设置插槽和本机 Agent 协议（开发版本）

`ctx.runtime.describe()` 返回当前宿主的能力名称。依赖新接口的插件应在激活时检查，不能将相同 API 格式版本当成所有可选能力都存在。

UI 服务声明 `metadata: {surface: 'settings'}`，返回 `{html,width,height}`，即可在设置侧栏注册入口。客户端仍通过隔离 MessagePort 的 `mewu.call()` 调用自己的服务；没有 Tauri、任意 IPC 或其他插件的接口。关闭页面、停用和更新撤销旧文档。

声明 `configuration.read` 后可用 `ctx.configuration.get(key)` 读取插件共享配置。写入需要 `configuration.write` 且调用来自该插件的活动设置文档，使用 `compareAndSet(key,expected,value)` 防止覆盖并发更改。普通 `ctx.storage` 继续按 Agent 作用域隔离，用于各角色的会话和记忆。

声明 `process.rpc` 后，`ctx.processes.resolve(candidates,files)` 可检测有界的安装路径；`${home}`、`${appData}`、`${localAppData}`、`${programFiles}` 是宿主展开的系统路径。`open({executable,args,cwd?,env?})` 启动固定本机可执行文件并返回 `{id,cwd}`。参数为数组，禁止 shell/自动安装器。`write(id,message)` 和 `read(id,timeoutMs)` 交换 JSON 行；读取超时返回 null，关闭/坏 JSON/资源超限返回失败。协议、请求 ID、重试和会话规则由插件实现。

本机程序按正常用户环境运行，拥有该用户权限；它不是 QuickJS 沙箱的一部分。运行包必须如实说明这一权限。进程句柄仅在本次调用有效，`close`、完成、取消、插件停用或卸载会清理它；不提供永久后台进程或跨插件句柄。

若声明 `media.read`，`write` 可附加 `{frameId,pointer,encoding:'base64'|'dataUrl'}` 绑定。宿主只把当前 Agent 输入中已授权的图片注入消息的 null JSON Pointer 占位，不向插件开放任意文件路径或他人素材。这个接口与所有平台协议无关。示例：[外部 Agent 插件](../../extensions/official/agent-connectors)。

让普通 Agent 按任务调用的能力使用 `ctx.tools.register`，提供明确的 `description` 和 `inputSchema`。需要本次用户授权选区的工具声明 `metadata: { requiresRegion: true }`；只有已明确授权区域的运行才会向模型提供该工具，插件也应拒绝缺少 `context.regionId` 的调用。注册工具不会创建截图工具条命令，也不会自行获得区域授权。

[`region-agent`](https://github.com/abnste/mewu-plugin-region-agent) 的 0.1.2 版将区域操作注册为工具，并保留独立 Agent 服务；Agent 在没有选区时仍可正常聊天。它继续使用 API 与 SDK 0.1.0 已有的注册接口，普通 Agent 的自动工具调度需要支持该能力的新版宿主。独立仓库附有 Windows 原生运行截图。更新插件时发布新的版本与 ZIP，不覆盖社区已发布的资产。

宿主按插件和 Agent 隔离持久存储。每条记忆可以独立保存，通过分页键目录检索；不存在固定 64 条的模型记忆槽位。`storage.list` 的 cursor 用于分页，prefix 用于过滤，不能互相替代。并发读改写使用 `storage.compareAndSet(key, expected, value)`：返回是否成功，`expected: null` 明确表示键不存在，不匹配已保存的 JSON null。单值及一次请求有资源限制，长期数据应拆分为独立记录。

模型调用使用宿主已配置的连接，插件不获得 API Key。区域观察返回宿主图片 artifact ID；将该 ID 放到 `ctx.model.invoke({ prompt, images: [artifactId] })`，由宿主提供真实图片。区域操作还需要用户此次明确发起的运行授权，仅有清单权限不够。坐标是观察图内的像素，不能传桌面全局坐标。

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
- `mewu.setDropTargets([{id,x,y,width,height}])` 声明文档内的放置区域。
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
PNG 缩略图（含标注），不支持预览的类型返回 `null`，不暴露本机文件路径。

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

当前区域自动操作的原生适配器仅支持 Windows。macOS 与 Linux 的截图交互不受影响，但它们的输入适配器尚未实现，会返回明确的不支持错误。Windows 区域须来自真实截图、位于同一个前台应用内容区，目标身份、显示器、选区与输入焦点会持续校验；Esc 或宿主停止可撤权。

Windows 的 layered 窗口同时具有 `WS_EX_TRANSPARENT` 时，鼠标输入由系统穿透到下方应用，这类叠层不会阻止区域操作，但仍可能出现在授权区域的截图中。宿主不承诺截图只包含目标应用的像素；观察和鼠标操作仍校验实际命中目标，检查不能锁住外部窗口的后续变化。

运行时没有 `process`、`require`、任意文件访问或直接网络接口。Node.js 的包不是可直接移植的宿主库；需要使用 SDK 服务或在插件包内提供兼容的 ESM 模块。

发布包的 `index.mjs` 是 `crates/mewu-extension/src/sdk.js` 的精确副本。修改嵌入式源后运行 `npm run sync`，`npm run check:sync` 与 `prepack` 拒绝两个版本分叉；npm 包不引用仓库外路径。
