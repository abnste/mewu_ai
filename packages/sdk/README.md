# @mewu/sdk

Mewu API 2 插件运行在独立 QuickJS worker 中。宿主提供 SDK 模块，安装插件不需要 Node.js；此 npm 包用于开发时的类型提示与契约测试。

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

普通 JavaScript 插件省略清单 `platforms`、市场条目填写 `platforms: ["any"]`，同一个 ZIP 即可在 Windows、macOS、Linux 使用同一套 SDK。仅整包确实依赖本机能力时，才填写 `platforms: ["windows-x86_64"]` 等允许清单；另两个目标为 `darwin-aarch64`、`linux-x86_64`。市场不得声明比包更广的平台，`any` 不能与具体平台混用。本地安装、启用、回退、依赖和运行也受宿主校验，无法仅绕过按钮安装。

同包只有个别功能受限时，用服务 `metadata.platforms` 限制该功能，其他 UI、记忆等继续跨平台。`metadata.requiresRegion: true` 的原生区域操作目前仅 Windows 可用，其他系统显示禁用原因；不能仅凭插件是 JavaScript 就宣称所有本机能力都已实现。处理函数内可按宿主提供的 `context.platform`（`windows` / `macos` / `linux`）适配可选行为。

可注册命令、工具、Agent、记忆、模型、通讯渠道、渲染器和 UI 服务。注册发生在激活阶段；返回的函数可注销服务，`ctx.onDispose` 可登记卸载清理。服务的设置、菜单位置等通过 descriptor 的 `metadata` 声明。API 类型以 `index.d.ts` 为准。

让普通 Agent 按任务调用的能力使用 `ctx.tools.register`，提供明确的 `description` 和 `inputSchema`。需要本次用户授权选区的工具声明 `metadata: { requiresRegion: true }`；只有已明确授权区域的运行才会向模型提供该工具，插件也应拒绝缺少 `context.regionId` 的调用。注册工具不会创建截图工具条命令，也不会自行获得区域授权。

[`region-agent`](https://github.com/abnste/mewu-plugin-region-agent) 的 0.1.2 版将区域操作注册为工具，并保留独立 Agent 服务；Agent 在没有选区时仍可正常聊天。它继续使用 API 2 与 SDK 0.1.0 已有的注册接口，普通 Agent 的自动工具调度需要支持该能力的新版宿主。独立仓库附有 Windows 原生运行截图。更新插件时发布新的版本与 ZIP，不覆盖社区已发布的资产。

宿主按插件和 Agent 隔离持久存储。每条记忆可以独立保存，通过分页键目录检索；不存在固定 64 条的模型记忆槽位。`storage.list` 的 cursor 用于分页，prefix 用于过滤，不能互相替代。并发读改写使用 `storage.compareAndSet(key, expected, value)`：返回是否成功，`expected: null` 明确表示键不存在，不匹配已保存的 JSON null。单值及一次请求有资源限制，长期数据应拆分为独立记录。

模型调用使用宿主已配置的连接，插件不获得 API Key。区域观察返回宿主图片 artifact ID；将该 ID 放到 `ctx.model.invoke({ prompt, images: [artifactId] })`，由宿主提供真实图片。区域操作还需要用户此次明确发起的运行授权，仅有清单权限不够。坐标是观察图内的像素，不能传桌面全局坐标。

当前区域自动操作的原生适配器仅支持 Windows。macOS 与 Linux 的截图交互不受影响，但它们的输入适配器尚未实现，会返回明确的不支持错误。Windows 区域须来自真实截图、位于同一个前台应用内容区，目标身份、显示器、选区与输入焦点会持续校验；Esc 或宿主停止可撤权。

Windows 的 layered 窗口同时具有 `WS_EX_TRANSPARENT` 时，鼠标输入由系统穿透到下方应用，这类叠层不会阻止区域操作，但仍可能出现在授权区域的截图中。宿主不承诺截图只包含目标应用的像素；观察和鼠标操作仍校验实际命中目标，检查不能锁住外部窗口的后续变化。

运行时没有 `process`、`require`、任意文件访问或直接网络接口。Node.js 的包不是可直接移植的宿主库；需要使用 SDK 服务或在插件包内提供兼容的 ESM 模块。

发布包的 `index.mjs` 是 `crates/mewu-extension/src/sdk.js` 的精确副本。修改嵌入式源后运行 `npm run sync`，`npm run check:sync` 与 `prepack` 拒绝两个版本分叉；npm 包不引用仓库外路径。
