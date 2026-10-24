# 独立可执行插件示例

每个子目录都是一个完整的 API 插件包，直接从 `@mewu/sdk` 导入宿主接口。它们没有插件 ID 特判、宿主源码补丁、截图 mock 或硬编码桌面目标；可以复制到自己的仓库修改并独立发行。

| 示例 | 注册的服务 | 行为 |
| --- | --- | --- |
| [`region-agent`](https://github.com/abnste/mewu-plugin-region-agent) | tool、agent | 从原有对话条发起任务；观察真实选区 → 模型决定一个动作 → 原生操作 → 新截图反馈，最多 30 步；也能作为普通对话 Agent 使用 |
| [`journal-memory`](https://github.com/abnste/mewu-plugin-journal-memory) | memory | 按 Agent 隔离独立记忆记录，原子 CAS 分配 ID，分页检索、删除，无固定总条数 |
| [`interactive-widget`](https://github.com/abnste/mewu-plugin-interactive-widget) | renderer、ui、command | 返回透明 HTML/SVG 圆角尺，内容可交互，由宿主空间对象提供移动等控件 |
| [`region-actions`](region-actions) | command | 在选区工具条上注册复制与贴图命令，通过 `ctx.actions.invoke` 请求宿主执行与第一方工具条一致的区域动作 |

区域操作仅支持 Windows 的原生适配器。必须由用户在当前真实截图选区上发起；插件清单中的权限不等于此轮屏幕操作授权。输入焦点、目标应用身份、显示器及选区会持续核验。macOS 和 Linux 尚未实现输入适配器，不能将跨平台 JavaScript 等同于跨平台输入支持。

区域插件的 `task` 来自用户输入；模型只获得宿主授权的区域 PNG。示例代码不持有 Key，不访问本机路径或直接启动程序。每次动作后重新观察，界面变化由新截图进入下一轮，Esc 或宿主停止会撤销后续动作。

从 Mewu 仓库根目录使用发行工具打包，例如：

```powershell
node tools/extension-market/cli.mjs pack extensions/examples/region-agent --out D:/packages/region-agent-0.1.2.zip
```

该命令需要已构建、契约一致的 `mewu-plugin-validator`；构建和 GitHub Release / 社区目录投稿流程见 [发行工具文档](../../tools/extension-market/README.md)。包根必须直接包含 `mewu-plugin.json` 与入口模块。不要将开发工具、测试文件、node_modules、密钥或外层目录装入 ZIP。

[SDK 类型与开发说明](../../packages/sdk/README.md) 中列出完整服务接口。`packages/sdk` 的 Node 测试只使用明确的 mock host 验证协议与示例逻辑，不是原生屏幕、模型或跨平台验收的替代品。
