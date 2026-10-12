# 本机开发检查

使用现有 Node.js 启动，不安装新依赖：

```powershell
node scripts/dev-check/server.mjs serve 49370
```

打开 `http://127.0.0.1:49370`。日常改动按需运行类型检查、前端构建和勾选的相关测试，再由开发者检查真实浏览器交互并更新、启动本机 Windows。前端构建按钮只执行 Vite；类型检查单独执行，结果分别记录。面板不会自行安装、退出软件或触发三平台重测。

命令固定为当前仓库的 TypeScript、Vite 和 `apps/desktop/src` 已列出的 Node 测试。禁止任意 shell、命令参数、文件路径或工作目录。只监听 IPv4 localhost，写请求要求同源、准确 Host 和随机 CSRF token。日志及状态保存在忽略目录 `.private/dev-check`；仅展示经过凭据过滤的工具输出，不读取环境变量或外部回执正文。不要把该开发工具转发至公网。

每个检查记录开始与结束、退出码、耗时、Git HEAD 和实际文件指纹。文件和 Git 元数据变化触发指纹失效；日常轮询复用缓存，不重复读取全部源码。每项命令开始和完成仍强制校验一次，以确认过程中的源码变化。源码变化会标为过期。控制台重启后未取得退出码的任务标为中断，不能算通过。

浏览器 / Windows / Linux / Mac 结果由主开发者绑定真实私有 JSON 回执登记。例如：

```powershell
node scripts/dev-check/server.mjs record --lane windows --status passed --source <40位源码提交> --kind windows-update --evidence .private/remake-review/<本轮真实回执>.json
```

lane 为 `browser/windows/linux/mac`，status 为 `pending/running/passed/failed/blocked`；kind 为 `browser-gui/user-windows/windows-update/docker-gui/mac-gui/build`。回执必须位于当前仓库 `.private` 内，且是可解析 JSON；面板记录其 SHA256 和大小。状态是开发者报告，工具不自行推断回执的验收结论，更不把仅构建成功当作 GUI 成功。CLI 登记可与控制台同时使用，但多个登记命令应顺序执行。

`browser-gui` 登记还必须提供 `--fingerprint <面板完整64位指纹>`，且与当前源码精确一致，并通过 `--url` 指定本机 Vite 的 `/.private/remake-review/snapshot-ui-fixture.html` 或当前仓库对应的 `/@fs/` 同名路径；不接受旧仓库地址、URL 凭据或查询参数。Mac 只读健康摘要来自 `.private/dev-check/mac-health.json`，限定字段和枚举、两分钟过期，不读取 SSH/VNC 凭据；连通性不等于功能通过。

发布时再统一版本运行 Linux Docker 与内网 Mac，并取得用户 Windows 验收；现有正式发布契约保持独立。此面板不是发布授权，也不会替代平台安装、更新和功能证据。三平台自动执行适配器尚未接入的阶段保持待执行，不假造连接或通过。

运行控制台安全检查：

```powershell
node --test scripts/dev-check/server.test.mjs
```

Windows 小更新使用 `update-windows.ps1`，显式指定已安装源码、候选源码与候选 SHA256。它正常退出旧程序、做一次冷备份、替换并启动；用户负责实机界面验收。数据模型、迁移、凭据与更新器改动不能使用此 UI 更新入口。入口涉及 `main.rs` 的窗口生命周期修改时，开发者必须先审查并提供绑定前后 Git blob 的私有 JSON 回执，不能把整个宿主文件无条件加入白名单。

候选来自另一份干净工作树时，传入 `-SourceRoot` 指定它的 Git 根目录。候选、回执和源码检查都绑定该目录；开发检查工具可单独更新，不修改已经编译冻结的产品源码。PowerShell 必须显式调用 Git 可执行文件，避免辅助函数与 Git 命令同名造成递归。

本机增量构建可用 Cargo `daily` 配置：`tauri build --no-bundle --ci -- --locked --profile=daily`。首次需建立独立缓存，后续复用增量编译；运行时仍启用优化，不启用调试断言。正式发布继续使用 `release` 的完整优化与安装包构建。不要把本机更新当作公开发布。
