# 本地插件与社区目录校验

这个工具离线检查 Mewu API 1 插件和社区目录，并将每个插件一个条目的源目录合并为宿主可读取的 JSON。校验直接调用应用的 Rust 解析器，支持 `selection.workflow`、`selection.drawing-tools`、`selection.ocr`、`selection.scroll`、`selection.translation`、`selection.pin`、`memory.provider`、`input.speech-to-text`、`selection.codes`、`artifact.video-trim`、`recording.audio`、`artifact.video-gif`、`selection.recording` 十三种贡献点；没有另一套 JavaScript schema，也不运行插件。`selection.pin` 只声明入口标识与标题，不接受脚本或任意窗口 URL。`input.speech-to-text` 只接受 `engine: "windows.sapi"`；声明校验不打开麦克风，也不证明当前系统有可用识别器。`selection.codes` 只接受 `engine: "rxing"`，不接受自定义命令、远端地址、自动打开或格式列表；校验清单不会执行识别。`artifact.video-trim` 只接受 `engine: "windows.media-editing"`，校验时不读取视频或启动媒体引擎；清单不包含范围、路径或编解码器命令。`recording.audio` 只接受 `engine: "windows.wasapi"`，不接受设备、麦克风开关、自动监听或命令；校验与安装均不打开音频设备。`artifact.video-gif` 只接受 `engine: "windows.media-editing-gif"`，不接受路径、编码参数或自动导出；清单校验不代表系统已具备经验证的媒体导出能力。

兼容第十四种贡献点 `agent.visual-annotations`，仅接受 `id`、`title` 与 `engine: "host.vector-v1"`。原位作答、图片标注和视频理解现由应用核心提供，旧官方清单只保存在 [迁移目录](../../apps/desktop/plugins/migrations/official-annotations-1.0.0.json)，不作为当前插件市场条目发布。社区清单仍可使用自己的非 `mewu.` 标识声明此兼容类型；停用或卸载插件不控制核心标注能力。拒绝其他引擎、对象形式的 engine、自定义提示词、权限、脚本和自动执行等字段。验证不会调用模型或生成标注。

绘制与完整屏幕录制现为核心自带能力。旧官方绘制、录制与三个拆分清单保留在 `plugins/migrations/`，只作兼容证据，不应发布成当前目录条目。第三方仍可使用相同严格贡献类型，`selection.recording` 的 `engine` 仅为 `windows.wgc-mf`；校验不会打开设备或执行录制。

## 准备

需要 Node.js 22 或更新版本，以及本仓库的 Rust/Tauri 构建环境。此工具没有新增 npm 或 Cargo 依赖。从仓库根目录执行：

```powershell
cargo build -p mewu-desktop --bin mewu-plugin-validator --locked
node tools/plugin-catalog/cli.mjs --help
```

默认使用 `target/debug/mewu-plugin-validator.exe`（其他系统无 `.exe`）；设置了 `CARGO_TARGET_DIR` 时使用该目录。可以指定 `--validator`，或设置 `MEWU_PLUGIN_VALIDATOR` 为独立校验器的绝对路径。不要使用桌面应用 EXE 代替。

该二进制通过 `#[path]` 编译应用现有的 `plugins.rs` 和 `plugin_sources.rs`，不会启动 Tauri、创建窗口、读取应用数据、连接网络或安装插件。每次结果带插件解析器、目录解析器与 core 模型/连接验证四份源文件的编译时 SHA-256，Node 与工作区原始文件字节比较；文件改变后必须重新构建校验器。这个指纹检测版本失配，不是发布者签名，也不验证一个来历不明的校验器。

贡献点类型、严格字段和固定引擎均以 `plugins.rs` 为真源；没有另行生成或维护的 JSON Schema。新增批注类型已改变该源文件指纹，旧校验器必须重建。待校验的官方清单仍作为原始输入读取，不以文件名或编译时内置清单代替当前字节。

## 校验插件

```powershell
node tools/plugin-catalog/cli.mjs manifest apps/desktop/plugins/examples/selection-workflow/mewu-plugin.json
node tools/plugin-catalog/cli.mjs manifest apps/desktop/plugins/official-ocr.json --bundled
```

普通 `manifest` 拒绝保留的 `mewu.` 标识。`--bundled` 只用于仓库内置清单的格式检查，不能让第三方文件获得官方来源权限。安装和运行授权仍由应用检查。

原始字节会直接交给严格解析器，保留重复字段、错误 UTF-8、BOM 等错误，不会先由 `JSON.parse` 消除它们。格式与限制见 [插件 API 1](../../apps/desktop/plugins/README.md) 和 [实际解析器](../../apps/desktop/src-tauri/src/plugins.rs)。除文档描述外，解析器还会检查规范化包体积、控制字符、贡献点重复 ID 和绘制工具重复项。

## 校验、生成目录

建议将每个提交保存为 `entries/<plugin-id>.json`，每个文件包含完整 `manifest` 和固定提交的 GitHub `source`。目录使用 JSON，与宿主格式相同，不引入 YAML 转换。

```powershell
node tools/plugin-catalog/cli.mjs entry tools/plugin-catalog/examples/entries/example.explain-chart.json
node tools/plugin-catalog/cli.mjs build tools/plugin-catalog/examples/entries --out mewu-catalog.example.json
node tools/plugin-catalog/cli.mjs catalog mewu-catalog.example.json
node tools/plugin-catalog/cli.mjs build tools/plugin-catalog/examples/entries --out mewu-catalog.example.json --check
```

**示例只用于离线格式演示。** `example/plugin` 与全零提交是占位值，没有对应的已核实远端插件。不要发布或安装示例生成的目录；提交真实目录前换成自己的公开仓库、完整提交和实际插件文件。

`build` 的行为：

- 只读取指定目录的直属 `.json` 普通文件；多余文件、子目录和链接明确报错，不会悄悄漏掉条目。文件名不影响插件身份，身份由清单决定。
- 最多 256 条，原始条目总计和最终生成文件均最多 2 MiB；逐条调用宿主校验器，再按插件 ID 排序，以 UTF-8 无 BOM 和 LF 输出。空目录对应合法空目录文件。
- 再次调用完整目录解析器，检查跨条目的重复 ID、GitHub 来源和完整提交。GitHub 文件路径实际还要求每段 ASCII 字符、末尾小写 `.json`，不接受查询参数、百分号或路径回退。
- 全部通过后才在输出同目录写临时文件、同步并替换。校验或替换失败不先删除旧目录；不能把输出放在条目目录内。崩溃可能留下 `.mewu-catalog-*.tmp`，工具不扫描删除用户文件，也不承诺文件系统断电事务。
- `--check` 只比较完整生成字节，缺失或不一致返回失败，不写文件。`--json` 输出一条机器可读结果；成功退出码 0，输入/目录错误 1，用法或校验器错误 2。

离线通过仅证明此版本宿主接受结构。它不证明 GitHub 仓库或提交存在、远端字节与目录一致、发布者拥有版权、提示词有用或没有恶意。应用安装前仍读取固定提交的真实文件并让用户检查；目录维护者仍需审阅来源、许可与实际行为。

## CI 使用

先由可信代码构建独立校验器，再对提交的 JSON 数据运行 `entry` / `build --check`。只校验条目的任务不需要网络或凭据。不要在拥有发布密钥的任务里检出并执行外部 PR 提供的脚本；发布流程、目录仓库和远端来源核验尚未配置，本工具不会替你创建或发布它们。

```powershell
$env:MEWU_PLUGIN_VALIDATOR = (Resolve-Path target/debug/mewu-plugin-validator.exe).Path
node --test tools/plugin-catalog/test.mjs
```

未设置 `MEWU_PLUGIN_VALIDATOR` 时，仅运行文件/进程层测试，真实 Rust 解析器黑盒测试明确跳过。完整验收必须设置它。测试仅使用合成 JSON 与临时目录；可用 `MEWU_PLUGIN_TEST_TMP` 指定测试临时目录的父目录。

## 参考社区机制

2026-10-08 核对：市场应用 [dsh-market/dsh-market，a355401e](https://github.com/dsh-market/dsh-market/blob/a355401e64c8e86f79e28d5c88ee2403527cfe08/README.md) 明确从独立目录读取列表，插件收录不向市场应用仓库提交。

实际目录 [awesome-dsh-plugin，9e5c9f34](https://github.com/awesome-dsh-plugin/awesome-dsh-plugin/blob/9e5c9f341a35042d3a305e0e54ecfcbb507630e6/contributing.md) 采用每插件一个 YAML 条目，PR 最多新增三项；自动检查安装清单、仓库条件与生成结果，维护者仍阅读源码核对描述。Mewu 借鉴独立条目和审核流程，不照搬其 npm、tarball、`dsh.bundle` 或可执行扩展契约。

它的 [PR 检查](https://github.com/awesome-dsh-plugin/awesome-dsh-plugin/blob/9e5c9f341a35042d3a305e0e54ecfcbb507630e6/.github/workflows/pr-check.yml) 专门防止条目落错目录、扩展名不对却校验全绿；这里对条目目录的异常文件直接报错。[带凭据的 gate](https://github.com/awesome-dsh-plugin/awesome-dsh-plugin/blob/9e5c9f341a35042d3a305e0e54ecfcbb507630e6/.github/workflows/pr-gate.yml) 执行可信基础分支代码，将 PR 条目作为数据读取。这个隔离原则适用于将来的目录 CI；本地格式校验不能替代人工审查。

进程通过 Node 标准库 [`spawn`](https://nodejs.org/api/child_process.html#child_processspawncommand-args-options) 直接执行，禁用 shell、限制输出和等待时间；生成文件使用同目录 [`rename`](https://nodejs.org/api/fs.html#fsrenameoldpath-newpath-callback)。Rust 独立目标使用标准 [Cargo binary target](https://doc.rust-lang.org/cargo/reference/cargo-targets.html#binaries)。

## 模型连接模板

`model.connection` 和模块标签由同一个宿主解析器校验。可验证 [模型连接示例](../../apps/desktop/plugins/examples/model-connection/mewu-plugin.json) 或 `official-provider-*.json`（需 `--bundled`）。贡献不能包含密钥、headers、可执行代码或未知高级参数；HTTPS、本机 HTTP、三协议、认证方式、参数范围均在纯解析中检查，不连接端点或调用模型。模板示例只在用户手动填写自己的服务地址与密钥后可用。安装模板不改变已有用户连接。
