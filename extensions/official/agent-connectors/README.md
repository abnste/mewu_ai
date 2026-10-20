# 外部 Agent

一个独立的 Mewu 插件，在设置侧栏注册“外部 Agent”，在原 Agent 选择器中提供 Codex、MiniMax Code、WorkBuddy / CodeBuddy 和 Hermes。

协议、安装路径规则、配置页面、模型查询、会话恢复和消息转换都在本插件中。宿主只提供设置插槽、隔离配置、受控的本机进程及 JSON 行通信；不包含这些平台的专用适配分支。需要包含这些新接口的 Mewu 开发版本，当前公开版 1.0.3 尚不支持。

## 使用

1. 安装插件，打开设置 → 外部 Agent。
2. 选择平台，自动检测或填写运行程序。Node/Python CLI 应选择本机运行程序，将入口脚本放在 JSON 启动参数的第一项。
3. 点击测试连接，选择服务返回的模型并保存。留空模型时使用客户端默认值。
4. 在设置 → Agent 与连接中，将角色的 Agent 提供方绑定到此插件的对应服务，然后从原对话条发送消息或截图。

插件不复制第三方的登录密钥。外部程序按其正常用户环境运行，继承其登录、代理、工具和原生运行时配置；`process.rpc` 是运行本机程序的权限，不是操作系统沙箱。启动参数使用数组传递，不通过 shell，也不自动下载客户端。

## 平台接口

| 平台 | 接口 | 说明 |
| --- | --- | --- |
| Codex | `codex app-server --listen stdio://` | 官方客户端登录、账户与模型检测，持久线程恢复。默认采用只读 sandbox。 |
| MiniMax Code | `mcode acp` | 官方 CLI 的 ACP 服务；不将桌面应用的私有令牌接口冒充正式 Agent 协议。 |
| WorkBuddy / CodeBuddy | 桌面内置 `--serve` HTTP ACP；或 `codebuddy --acp` | 自动检测桌面程序及 CLI，桥接逻辑完全在插件里。需要使用对应官方客户端/CLI 的登录。 |
| Hermes | `hermes acp` | 需要安装 ACP 支持；沿用 Hermes 的身份、记忆、技能和模型配置。 |

配置全插件共享；会话按角色、Mewu 会话及连接配置隔离。支持恢复时复用远端会话；不支持时发送 Mewu 已投影的对话历史。每轮进程归该调用所有，结束、取消或插件撤权会关闭。图片通过本轮原生素材句柄注入协议消息，不允许插件传入任意本机图片路径。

## 当前边界

- 目前适配 ACP v1 和 Codex App Server；未宣称支持所有未来协议版本。
- 测试连接会完成握手和模型/会话检测，可能在第三方创建一个空测试会话；不会发送模型提示词。“服务可连接”表示协议可用，不能证明账户额度或每个模型均可执行。对话中的登录失败会明确提示。
- 外部 Agent 请求客户端文件系统、终端或额外操作授权时，当前客户端返回不支持或拒绝，不会自动批准。已有服务自身的工具仍遵守其自身配置。
- 单次执行预算最多 300 秒。进程/协议错误会关闭进程并报告失败，不自动重发已发送的任务。
- 普通对话、图片和公开思考流已接入；未实现第三方专用视频上传和完整原生工具审批界面。
- Windows 本机 Codex、Hermes 已通过真实运行时的握手、文字回复、关闭进程后的两轮会话恢复与图片传递。WorkBuddy 当前桌面 CLI 的 stdio 不工作，HTTP ACP 桥已连通；本机模型调用返回需要登录，因此尚未通过完整对话验收。MiniMax CLI 和 Mac/Linux 尚未实机验收，不能以协议测试替代。

## 开发与打包

入口为 `index.mjs`，无需 Node 依赖随插件安装。使用项目 SDK 打包器生成安装 ZIP，README 和测试文件不放入运行包。

```sh
node tools/extension-market/cli.mjs pack extensions/official/agent-connectors --out out/agent-connectors-0.1.1.zip --file profiles.mjs --file rpc.mjs --file adapters.mjs --file view.mjs --file workbuddy.mjs --validator /path/to/mewu-plugin-validator
```

一手参考：[ACP v1](https://agentclientprotocol.com/protocol/v1/overview)、[Codex App Server](https://developers.openai.com/codex/app-server)、[MiniMax Code](https://github.com/MiniMax-AI/minimax-code)、[CodeBuddy ACP](https://www.codebuddy.ai/docs/cli/acp)、[Hermes ACP](https://hermes-agent.nousresearch.com/docs/user-guide/features/acp/)。
