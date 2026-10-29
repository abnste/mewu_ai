# 插件外观协议

Mewu 1.2.0 与 SDK 0.3.0 提供 `ui.appearance.v1`。插件声明外观贡献，宿主消费语义令牌；不注入 CSS，不访问宿主 DOM，不改变窗口布局。市场分类和注册枚举不代表接口已经可用，作者应读取 `runtime.describe()`。

## 注册

清单声明 `permissions: ["ui.theme"]` 和 `requiresCapabilities: ["ui.appearance.v1"]`。后者在激活前检查，缺少能力时不运行插件代码；旧清单可省略它。

```js
import { definePlugin } from '@mewu/sdk';
export default definePlugin(async ctx => {
  const contract = await ctx.ui.themes.describe();
  ctx.ui.themes.register({
    id: 'my-theme', title: 'My theme',
    theme: {
      version: contract.version, base: 'dark',
      colors: { accent: '#80D6C5', onAccent: '#102720' }
    }
  });
});
```

同一插件可注册多个主题，身份由插件 ID 与服务 ID 共同确定。安装不会自动切换外观；用户在“设置 → 通用 → 主题”选择。修改定义并通过正常版本更新发布，不需要修改宿主。

## 共同契约

[appearance-contract.json](../crates/mewu-extension/src/appearance-contract.json) 定义颜色名称、CSS 语义别名和明暗默认值。SDK 的 `appearance.json` 与宿主默认令牌由同一来源生成，SDK 的 `check:sync` 核对三者。原组件直接消费这些令牌，不叠加另一套组件颜色覆盖。

`base` 为 `light` 或 `dark`；未指定颜色继承该基色。颜色只接受 `#RRGGBB`，未知名称、表达式、URL、字体和脚本均不进入宿主样式。

背景仅接受 `workspace`、`settings`、`composer` 三个作用域，形状为 `{png,fit}`。`png` 是不带 Data URL 前缀的规范 Base64 PNG，每张编码不超过 256 KiB；宽高各不超过 4096，总像素不超过 8,000,000。`fit` 为 `cover`、`contain` 或 `repeat`。不接受 SVG、外部请求或动画接口。

背景只改变宿主表面。截图原始像素、黑板文档底图、笔迹、视频和生成内容保留原文档颜色。主题不改变原生透明性、几何、权限或用户文件。

## 生命周期

第一方窗口在内容挂载前读取经过验证的外观投影，随后监听变化。投影不包含安装路径、凭据或其他服务。设置、对话条、绘制和录制控制条、贴图控件、悬浮会话消费同一套语义令牌。

每个插件按修订单调合并；迟到旧消息不能覆盖较新的撤销。更新替换定义，停用或卸载立即恢复默认值。用户选择身份保留，同身份重新启用或重装可恢复；多个主题不混叠。重启先恢复选择再挂载内容，避免先显示默认外观再切换。

## 插件界面

隔离界面通过 MessagePort 接收只读 `appearance` 快照。既有 `colors.text/field/line/muted/accent` 保留，新增完整语义 `tokens`。

```js
await mewu.ready;
const stopFollowing = mewu.appearance.follow();
// 拆除自己的控件时调用 stopFollowing()，恢复原先内联值。
```

作者也可用 `current()` 或 `onChange(callback)` 自行呈现。关闭界面撤销监听，外观消息不提供其他插件或 Tauri IPC 权限。生成文档不被强制套色，作者可自行选择是否消费主题。

参考：[DSH 样式所有权](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/web-styling.md)、[DSH 主题快照](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-theme/README.md)、[VS Code 主题贡献点](https://code.visualstudio.com/api/references/contribution-points#contributesthemes)。Mewu 保留 Tauri、Rust 与隔离 JavaScript 运行时，不引入 Electron。
