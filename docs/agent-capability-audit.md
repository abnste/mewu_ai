# 原生 Agent 与 Pi 的能力差距

核查日期：2026-10-28。Mewu 源码基线：`0cf95b27f7d7a5323f26245135177ba239e823a8`。
Pi 最新正式发行版：`v1.4.0`，固定提交
`4a080252f06caf63a525d7873c5d94d3c23887e0`。

以下矩阵记录上述旧基线的问题，不作为后续代码的状态表。该基线没有 Pi 的完整本机执行能力，
新增 `system_info` 仅解决了一个特定问题。

后续修复已移除该专用工具，新增独立的 `mewu-agent-tools` 通用执行模块、原生进程树管理、
工具原始参数和结果的持久上下文，以及技能脚本执行路径。当前实现及保留边界见
[Agent runtime](agent-runtime.md)。源码、离线回归与真实模型/三端实测是不同证据；
本页不把其中任何一项冒充全部验收完成。

## 能力逐项核对

| 能力 | Pi v1.4.0 | Mewu 当前原生 Agent | 结论 |
| --- | --- | --- | --- |
| 通用命令执行 | `bash`；另有 Windows `powershell` 工具，共用命令执行接口 | 普通 Agent 没有通用命令工具；插件进程与 Hooks 命令执行不是原生 Agent 的执行接口 | 缺失 |
| 通用文件操作 | `read`、`write`、`edit`，支持工作目录内相对路径及绝对路径 | `skill_read` 只能读技能目录，`plugin_project` 只能操作插件工程 | 缺失 |
| 目录和代码搜索 | 另提供 `ls`、`find`、`grep`；也可组合 shell 命令 | 没有相应本机通用工具；记忆或会话搜索不能替代文件搜索 | 缺失 |
| 执行工作目录与环境 | session cwd、工具 cwd、shell 解析、子进程环境、可替换 operations | 普通原生 Agent 没有统一 cwd/执行环境；现有插件工程目录承担的是另一种职责 | 缺失 |
| 命令输出与取消 | 输出增量、退出码、可选超时、取消进程树 | 已有普通 run 取消与外部进程 Job/process-group 管理，但没有接到原生通用执行链 | 部分基础设施存在，Agent 能力缺失 |
| 大输出继续读取 | 有界模型输出，截断说明和完整输出文件路径 | 现有原生工具结果超过预算会错误或投影省略；没有对应完整命令输出文件的分页读取链 | 缺失 |
| 工具失败后的修正 | 工具错误作为结果进入后续模型轮；取消和错误终态有明确区别 | 已有参数错误/部分已观察失败反馈；执行器返回 `Err` 时仍可能直接结束 run，不能声称所有失败都可修正 | 部分实现 |
| 长任务运行策略 | 根据工具结果、排队输入、续行条件继续；可由运行时钩子调整 | 原生循环固定 12 模型轮、32 工具调用；缺少相同层次的任务调度策略接口 | 部分实现 |
| 执行中追加指令 | 独立 steering 与 follow-up 队列，有消费时机和取消处理 | 没有等价的运行中纠偏与完成后排队机制 | 缺失 |
| 后续请求的工具上下文 | session 保存 assistant tool calls 与 tool results，按活动分支重建模型上下文 | 普通历史请求主要重建 user/assistant 文本及附件；独立工具日志没有完整接回普通后续请求 | 关键缺口 |
| 上下文压缩 | 阈值压缩、溢出恢复、分支摘要、文件操作累积和持久会话条目 | 有发送前摘要、批次后压缩、原文保留与摘要检查点；主要依靠估算和手设窗口，没有完整对齐溢出恢复/分支机制 | 部分实现 |
| 会话恢复与分支 | 持久树、活动分支、tree/fork/clone/resume | 有会话历史、工具回执、续答和摘要恢复；没有等价的树状执行历史及分支语义 | 部分实现 |
| Skills | 元数据渐进加载、支持文件、通用工具执行脚本、用户与项目技能发现、显式技能命令 | 有元数据和技能文件读取；没有通用执行工具，不能完成需要 shell/本机文件处理的普通技能流程 | 部分实现 |
| 项目上下文 | cwd 关联的 AGENTS/CLAUDE/SYSTEM 等上下文与项目资源加载机制 | 普通原生 Agent 没有等价的项目上下文发现和加载协议 | 缺失 |
| Agent 运行时扩展 | context/request/turn/tool 等运行时接口，可替换工具 operations，提供 SDK 与 RPC | Mewu 可注册 Agent/记忆/模型/工具服务，并可接外部客户端；不是同等完整的原生循环中间件或共享执行底座 | 部分实现 |
| 生命周期事件 | Agent/turn/tool/compaction/queue 等结构化生命周期及拦截接口 | Hooks 有提示词预处理、回复及工具完成/失败事件；当前 `tool_settled` 的通知不能替代通用 before/after tool 拦截 | 部分实现 |
| CLI 与程序化控制 | 终端、print、JSON、RPC、SDK 共用 Agent/session 机制 | 已有真实 Mewu CLI、聊天和应用控制接口；缺少上述队列/分支/通用本机操作的对应运行时协议 | 部分实现 |

Pi 工具注册表共有八种通用工具：`read`、`bash`、`powershell`、`edit`、`write`、
`grep`、`find`、`ls`。默认编码工具组合是 `read / bash / edit / write`，
不是说八种工具在每个平台均默认启用。Mewu 原生工具中的记忆、技能、会话、
插件工程与特定系统查询，不等价于这八种通用工具。

## 两个容易被掩盖的根因

### 把能力缺失误报成权限问题

模型只有宿主真实提供的工具。若没有通用执行接口，修改提示词不会获得系统命令能力。
让模型先创建插件再通过插件进程能力完成普通本机任务，错误地把用户任务变成了开发任务。
为每种查询添加专用工具，同样没有补齐通用能力。

Pi 的启用工具使用启动账户的操作系统权限；cwd 不是文件系统沙箱。
项目 trust 决定是否加载项目资源，也不是逐条命令的权限边界。
Mewu 后续协议必须明确实际访问范围、工作目录、进程环境和撤销生命周期，不能一边提供
任意 shell，一边声称它只能访问某个工程目录。

### 保存了工具日志，不代表模型具有连续的执行上下文

Mewu 有持久工具回执，但 `ai::canonical_messages` 普通会话分支只重建消息正文和附件。
`agent_context::transcript` 会把工具步骤摘要带入压缩输入，却不等于完整重建原始工具调用
和返回；`session_read` 读取的也是会话文本。下一次用户发言时，不能依赖模型凭上一条
自然语言总结，推测上次命令、路径、退出码或文件变更。

修复应从可持久重建的执行事件开始，同时保留现有未知结果、取消、原生对象、凭据和
插件撤权边界。不能通过重放已有副作用来恢复上下文。

## 重构的验收门

实现进度应按这些真实行为验收，不以工具数量、测试数量或单个系统版本查询代替：

1. 通过通用 shell 查询系统信息；同一工具还能执行另一个不相关命令，不生成插件。
2. 在独立测试目录完成创建、读取、精确修改、搜索、执行脚本和删除；中文、空格路径及
   相对路径按 cwd 正常工作，错误 cwd 返回实际错误。
3. 命令非零退出后，模型看见退出码和实际错误，自行决定修正；未知写入不得自动重放。
4. 长输出保留完整内容，模型可分段继续读取；stdout/stderr 增量和最终退出状态可追溯。
5. 在执行中取消，实际 shell 及其测试子进程退出；取消后不开始下一工具或模型请求。
6. 下一用户轮次及正常重启后仍可获得必要执行上下文，不靠长期记忆伪装会话恢复。
7. 运行中用户纠偏与 follow-up 队列有明确消费规则；压缩保留完整工具批次。
8. 普通带脚本 Skills 可以通过通用工具运行；技能本身不凭空增加授权。
9. 替换引擎或插件实现可复用协议中的执行能力和生命周期，不为某个 Agent 品牌加核心分支。
10. Windows、macOS、Linux 分别验证 shell 解析、编码、路径、进程清理；原有截图绘制另做回归。

长期记忆、截图、视频标注和黑板是 Mewu 的独立产品能力；保留这些能力不能抵扣以上缺口，
也不应为了接入通用执行而丢掉这些能力。

## 关键源码依据

Mewu：

- [原生工具注册和执行](../apps/desktop/src-tauri/src/agent_development.rs)
- [模型工具循环](../apps/desktop/src-tauri/src/agent_loop.rs)
- [普通历史请求重建](../apps/desktop/src-tauri/src/ai.rs)
- [上下文摘要输入](../apps/desktop/src-tauri/src/agent_context.rs)
- [会话文本读取](../apps/desktop/src-tauri/src/agent_session.rs)
- [固定运行预算](../crates/mewu-core/src/journal_types.rs)
- [现有外部进程生命周期](../apps/desktop/src-tauri/src/mcp_runtime.rs)
- [事件触发器](../apps/desktop/src-tauri/src/hooks/mod.rs)
- [扩展契约](../packages/sdk/index.d.ts)

Pi（全部固定在同一正式版提交）：

- [通用工具注册](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/src/core/tools/index.ts)
- [命令执行](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/src/core/tools/bash.ts)
- [Agent 调度、队列及取消](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/agent/src/agent.ts)
- [会话、压缩、重试与资源协调](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/src/core/agent-session.ts)
- [会话存储](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/src/core/session-manager.ts)
- [Skills](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/docs/skills.md)
- [运行时权限说明](https://github.com/earendil-works/pi/blob/4a080252f06caf63a525d7873c5d94d3c23887e0/packages/coding-agent/docs/security.md)
