# 外部 Agent

一个独立的 Mewu 插件，在设置侧栏注册“外部 Agent”，在原 Agent 选择器中提供 Codex、MiniMax Code、WorkBuddy / CodeBuddy 和 Hermes。

协议、安装路径规则、配置页面、模型查询、会话恢复和消息转换都在本插件中。宿主只提供设置插槽、隔离配置、受控的本机进程及 JSON 行通信；不包含这些平台的专用适配分支。对话条内的引擎与模型选择需要 Mewu 1.0.10 或更新版本；旧宿主仍可在 Agent 设置中绑定引擎。

## 使用

1. 安装插件，打开设置 → 外部 Agent。
2. 选择平台，自动检测或填写运行程序。Node/Python CLI 应选择本机运行程序，将入口脚本放在 JSON 启动参数的第一项。
3. 点击测试连接，选择服务返回的模型并保存。Codex 留空时采用当前模型目录声明的默认模型，避免本机旧默认值失效。
4. 在对话条的 Agent 菜单选择执行引擎，再从模型菜单选择该客户端实际提供的模型。原角色的人格、灵魂设定不变；引擎绑定属于角色，模型选择属于当前角色的当前会话。

自动检测保存为动态定位，客户端升级后重新定位；手工编辑程序或启动参数会切回手动模式，插件不会覆盖用户指定的路径。支持原生 CLI、官方 Node/npm 安装及明确的用户自定义运行程序；不启动 cmd/PowerShell 包装器。Node 脚本与参数分别传递，支持空格及中文目录。不同安装器、登录状态和模型权限仍需分别验证，自动检测不代表所有客户端或模型都可用。

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
- 选择 ACP 模型时先由客户端确认；拒绝的选择不会覆盖原模型。对话中的登录、额度和服务错误使用不含敏感信息的提示，不将客户端连接成功等同于模型可用。
- 单次执行预算最多 300 秒。进程/协议错误会关闭进程并报告失败，不自动重发已发送的任务。
- 普通对话、图片和公开思考流已接入；未实现第三方专用视频上传和完整原生工具审批界面。
- Windows 已实测 Codex、MiniMax Code 的原对话条发送、动态模型选择和实际回复；真实进程测试另覆盖两轮图像输入与会话恢复。macOS、Ubuntu 已实测 MiniMax Code 原对话条发送与实际回复，macOS 另验证了模型权限失败提示。Hermes 与 WorkBuddy 的既有覆盖范围保持原样：Hermes 在 Windows 通过两轮图像及恢复；WorkBuddy 桌面 HTTP ACP 已连通，但当前测试账号未完成模型登录，不能据此声称完整对话通过。

## 开发与打包

入口为 `index.mjs`，无需 Node 依赖随插件安装。使用项目 SDK 打包器生成安装 ZIP，README 和测试文件不放入运行包。

```sh
node tools/extension-market/cli.mjs pack extensions/official/agent-connectors --out out/agent-connectors-0.1.6.zip --file profiles.mjs --file rpc.mjs --file adapters.mjs --file view.mjs --file workbuddy.mjs --file catalog.mjs --validator /path/to/mewu-plugin-validator
```

一手参考：[ACP v1](https://agentclientprotocol.com/protocol/v1/overview)、[Codex App Server](https://developers.openai.com/codex/app-server)、[MiniMax Code](https://github.com/MiniMax-AI/minimax-code)、[CodeBuddy ACP](https://www.codebuddy.ai/docs/cli/acp)、[Hermes ACP](https://hermes-agent.nousresearch.com/docs/user-guide/features/acp/)。
