# JavaScript 插件发行与社区市场

Mewu API 2 插件在独立 QuickJS 进程中执行 ES Modules，通过 SDK 动态注册命令、工具、Agent、记忆、模型、渠道、渲染器和界面服务。这个工具负责把独立插件打成 ZIP、发布到 GitHub Release，再核验社区目录中的真实发行资产。它不启动插件、不调用模型、不执行作者安装脚本。

需要 Node.js 22+、GitHub CLI（仅 `publish` 需要），以及从可信 Mewu 源码构建的独立校验器。插件清单、目录条目、完整目录均把**原始 JSON 字节**交给宿主的同一 Rust parser；没有另一份 JavaScript 清单 schema。API 1 的声明插件工具仍在相邻 `plugin-catalog` 中，不能代替 API 2 校验器。

```powershell
cargo build -p mewu-desktop --bin mewu-plugin-validator --features catalog-validator --locked
$env:MEWU_PLUGIN_VALIDATOR = (Resolve-Path target/debug/mewu-plugin-validator.exe).Path
node tools/extension-market/cli.mjs --help
```

API 2 校验器额外返回 `extensionContractVersion: 2` 和 `extensionContractSha256`；指纹按照 `contract.rs`、`package.rs`、`sdk.js`、`extension_market.rs` 的原始字节，以 NUL 分隔计算。源码变化后必须重编译，不能用旧二进制冒充当前格式验收。指纹检测契约失配，不证明一个外来 EXE 的来源。

项目与图库使用独立 `mewu-projects.json`，原 `mewu-catalog.json` 的 schemaVersion 2 和安装来源保持不变。新校验模式为 `extension-project`、`extension-projects`、`extension-project-image`，使用生产 `extension_project.rs`；输出单独的 `projectContractVersion: 1` / `projectContractSha256`，不改变原四源指纹。

作者在独立公开项目里保留 `LICENSE`、与发行 ZIP 完全相同的清单及所有 JS/mjs 模块。提交 `projects/<plugin-id>.json`，字段为 `id/version/packageSha256/sourceRepository/sourceCommit/screenshots`；`sourceRepository` 是 `owner/repo`，`sourceCommit` 是完整不可变提交。每个插件使用自己的项目，安装 ZIP 仍来自原目录条目的发行来源。没有真实效果截图时用 `screenshots: []`。

截图必须是真实静态 PNG/JPEG/WebP，文件后缀与实际编码一致，每张最多 2 MiB、16–4096 像素边长、最多 8 Mi 像素，每个项目最多六张。每项填写 `path/sha256/size/width/height`，可选 `caption`；图片位于固定源码提交内，不允许任意外链、SVG、HTML、动画或符号链接。CI 检查 PNG CRC、实际原生解码、字节与尺寸，并逐字节比较固定提交中的源码模块和真实发行 ZIP，绝不运行作者代码。

```sh
node tools/extension-market/cli.mjs project projects/example.plugin.json --validator /path/to/mewu-plugin-validator
node tools/extension-market/cli.mjs verify-project projects/example.plugin.json --catalog mewu-catalog.json --validator /path/to/mewu-plugin-validator
node tools/extension-market/cli.mjs build-projects projects --catalog mewu-catalog.json --out mewu-projects.json --validator /path/to/mewu-plugin-validator
```

`build-projects` 在全部来源、模块和截图验证通过后原子替换侧边目录。失败保留旧文件；`--previous` 保留已有插件源码项目归属，`--check` 不写文件。`--offline` 仅用于本地结构检查，公开 CI 不得使用。额外原生测试入口是 `MEWU_PLUGIN_VALIDATOR=... node --test tools/extension-market/project-native-test.mjs`，缺校验器或指纹过期均失败，不跳过。

## From an independent project to a market PR

以下在一个工作目录执行，插件源码已位于 `my-plugin/`，包含上述清单、入口和许可。先把源码发布到自己的公开 GitHub 仓库 `YOUR_ACCOUNT/my-plugin`；替换账号、插件 ID、版本和分支名，平台与测试结果必须如实填写。开发机器需要 Rust/Cargo、Node.js 22+、Git/GitHub CLI 以及 [Tauri 构建前提](https://v2.tauri.app/start/prerequisites/)。SDK TGZ 只提供 SDK/类型，不包含打包 CLI 或原生校验器。

Windows PowerShell，使用默认 Cargo target 目录：

```powershell
git clone --branch next --single-branch https://github.com/abnste/mewu_ai.git ./mewu-host
cargo build --manifest-path ./mewu-host/Cargo.toml -p mewu-desktop --bin mewu-plugin-validator --features catalog-validator --locked
New-Item -ItemType Directory -Force ./packages | Out-Null
node ./mewu-host/tools/extension-market/cli.mjs manifest ./my-plugin/mewu-plugin.json --validator ./mewu-host/target/debug/mewu-plugin-validator.exe --json
node ./mewu-host/tools/extension-market/cli.mjs pack ./my-plugin --out ./packages/my-plugin-1.0.0.zip --validator ./mewu-host/target/debug/mewu-plugin-validator.exe --json
```

先从 Mewu 设置 → 插件 → 添加 → 安装 ZIP 插件进行本机验证。启用后，命令由声明的截图/空间入口运行；Agent 与记忆服务在对应 Agent 设置中选择。打包校验不能代替这些实际交互测试。macOS/Linux 将校验器文件名改为无 `.exe`，并使用系统对应的建目录命令；如设置 `CARGO_TARGET_DIR`，校验器路径也须对应调整。

以下只向作者自己的仓库上传包，向社区仓库提交条目 PR；不向 Mewu 宿主提交实现 PR，也不修改社区索引。`gh release create` 创建已发布版本，不能使用草稿：

```powershell
gh auth login
gh repo fork abnste/mewu-plugins --clone
git -C ./mewu-plugins switch -c add-yourname-counter
gh release create v1.0.0 --repo YOUR_ACCOUNT/my-plugin --title v1.0.0 --notes "Initial plugin release"
node ./mewu-host/tools/extension-market/cli.mjs publish ./packages/my-plugin-1.0.0.zip --repository YOUR_ACCOUNT/my-plugin --release-tag v1.0.0 --entry-out ./mewu-plugins/entries/yourname.counter.json --platform any --validator ./mewu-host/target/debug/mewu-plugin-validator.exe --json
$entry = Get-Content -LiteralPath ./mewu-plugins/entries/yourname.counter.json -Raw | ConvertFrom-Json
$sourceCommit = (git -C ./my-plugin rev-parse HEAD).Trim()
$project = [ordered]@{ id = $entry.id; version = $entry.version; packageSha256 = $entry.sha256; sourceRepository = 'YOUR_ACCOUNT/my-plugin'; sourceCommit = $sourceCommit; screenshots = @() }
New-Item -ItemType Directory -Force ./mewu-plugins/projects | Out-Null
[IO.File]::WriteAllText((Join-Path (Get-Location) 'mewu-plugins/projects/yourname.counter.json'), (($project | ConvertTo-Json -Depth 6) + "`n"), [Text.UTF8Encoding]::new($false))
node ./mewu-host/tools/extension-market/cli.mjs project ./mewu-plugins/projects/yourname.counter.json --validator ./mewu-host/target/debug/mewu-plugin-validator.exe --json
git -C ./mewu-plugins add -- entries/yourname.counter.json projects/yourname.counter.json
git -C ./mewu-plugins commit -m "收录 yourname.counter 1.0.0"
git -C ./mewu-plugins push -u origin HEAD
gh pr create --repo abnste/mewu-plugins --base main --head YOUR_ACCOUNT:add-yourname-counter --web
```

`sourceCommit` 必须已推送到该公开源码仓库，清单及所有包内模块必须与发行 ZIP 的字节相同。上例没有真实截图，明确保留空数组；需要展示效果时，在该固定提交中加入真实截图，并按前述字段填写数组。

在 PR 页面填写用途、权限、支持平台与实际测试结果后提交；PR 只改对应 `entries/<id>.json` 与 `projects/<id>.json`，不要编辑 `mewu-catalog.json` 或 `mewu-projects.json`。社区 [贡献规则](https://github.com/abnste/mewu-plugins/blob/main/CONTRIBUTING.md) 和 CI 核验通过、维护者合并后，主分支会生成两个公开索引。后续更新保留 ID/发行仓库/源码项目并增加新版本，不替换旧 Release 的资产。

## 打包与发行

清单格式和 SDK 用法见 [运行时源码](../../crates/mewu-extension/src/contract.rs) 与 [SDK](../../crates/mewu-extension/src/sdk.js)。包根是 `mewu-plugin.json`；`entry` 指向包内 `.js` 或 `.mjs` 模块。默认只打包清单和入口，其他导入模块逐项添加，防止无意收进本机文件。首版仅支持 JavaScript 模块；HTML、SVG 等界面内容由插件返回渲染数据，不把本机程序、npm 依赖树、密钥或原生安装脚本放进包中。

```powershell
node tools/extension-market/cli.mjs manifest D:/plugins/my-plugin/mewu-plugin.json
node tools/extension-market/cli.mjs pack D:/plugins/my-plugin --out D:/packages/my-plugin-1.0.0.zip --file lib/helpers.js --json
```

ZIP 使用固定时间、按路径排序的 stored 格式，同样输入产生同样字节，无新增 npm/Python/压缩器依赖。校验也接受普通 deflate ZIP。包最多 16 MiB、展开总计最多 8 MiB、最多 128 个 JavaScript 模块，单模块最多 1 MiB、清单最多 64 KiB。路径只接受 ASCII 常见文件名；拒绝大小写冲突、软链接、路径穿越、系统保留路径、加密、多卷、ZIP64 和额外字段。不要用其他打包器自动添加外层文件夹、目录条目或扩展信息。

先在**自己的独立 GitHub 仓库**创建公开发行版本，再明确运行上传命令：

```powershell
node tools/extension-market/cli.mjs publish D:/packages/my-plugin-1.0.0.zip --repository your-name/my-plugin --release-tag v1.0.0 --entry-out D:/catalog/entries/your-name.my-plugin.json --platform any --json
```

`publish` 不创建仓库、tag 或 release，也不覆盖资产。同名资产已经存在且字节一致时可重复执行；同名不同内容立即失败，需新版本。工具重新读取真实 GitHub 发行，校验资产 ID、来源、体积、GitHub 摘要（如果提供）和实际下载 SHA256，绑定清单身份后才输出条目。上传成功但后续网络失败时，保留真实发行资产、不伪造成功条目，重跑同一命令可验证恢复。条目文件名必须等于插件 `id.json`。

目录中 `platforms` 为 `any` 或 `windows-x86_64`、`darwin-aarch64`、`linux-x86_64` 的非重复列表。只有真实支持的系统才可填写；JavaScript 跨平台不表示依赖的屏幕输入等宿主能力已跨平台。包自身许可由作者声明，Mewu 源码许可不变。

## 核验与生成目录

```powershell
node tools/extension-market/cli.mjs verify D:/catalog/entries/your-name.my-plugin.json --json
node tools/extension-market/cli.mjs build D:/catalog/entries --out D:/catalog/mewu-catalog.json --json
node tools/extension-market/cli.mjs build D:/catalog/entries --out D:/catalog/mewu-catalog.json --check --json
```

每条记录包括 `id/name/description/tags/repository/version/releaseTag/assetName/sha256/size/hostApi/platforms`，`manifestPath` 如指定只能是根 `mewu-plugin.json`。完整索引为 `{"schemaVersion":2,"plugins":[...]}`，按 ID 排序，以 UTF-8 无 BOM、LF 输出。空目录合法，工具不会捏造种子插件。

`icon` 是可选的结构化 SVG 路径图标，格式见 [SDK 图标说明](../../packages/sdk/README.md)。`publish` 自动从包清单复制图标；旧包无图标时，可只在市场条目中补充，不更换 ZIP、版本或 SHA。包与条目都声明图标时必须一致。新版客户端读取 `mewu-catalog-v2.json`；原 `mewu-catalog.json` 保留不含图标字段的兼容目录，避免旧宿主的严格解析器拒绝整个市场。维护者用同一次已验证构建生成两份目录：

```powershell
node tools/extension-market/cli.mjs build D:/catalog/entries --out D:/catalog/mewu-catalog-v2.json --legacy-out D:/catalog/mewu-catalog.json --previous D:/catalog/mewu-catalog.json --json
```

`build` 默认实际核验每条 GitHub Release ZIP。`--offline` 仅做开发结构检查，结果明确标识 `releasesVerified:false`；公开目录发布不可使用它。`--previous` 指定可信旧目录，避免 PR 通过替换生成文件掩盖 ID 换源；已有 ID 的仓库不能悄悄变更。安装端仍要独立绑定已安装插件的来源并重新验包。对于移除后保留的永久 ID 所有权，目录维护者须保留历史或保留条目，不能只凭新 PR 的声明认可转移。

条目目录仅接受直属 `.json` 普通文件，最多 256 条，总计和生成索引各不超过 2 MiB；目录、软链接、杂项和重复身份会失败。全部格式、来源、网络、摘要核验通过后才在输出同目录写临时文件、同步、rename 替换；失败保留原索引。`--check` 不写文件。输出不能放在输入 entries 中。标准 GitHub 下载仅从 API 开始，人工处理 HTTPS 重定向，只允许 GitHub 的资产域名，认证头不随下载重定向转发。

## 社区仓库与 CI

[community-market](../../community-market/README.md) 可作为独立 `abnste/mewu-plugins` 仓库发布：`entries/<id>.json` 与 `projects/<id>.json` 供 PR 收录，`mewu-catalog-v2.json` 是新版安装索引，`mewu-catalog.json` 是旧版兼容索引，`mewu-projects.json` 是源码与图库索引。初始化模板的索引为空，只有真实发布并验证过的资产才可加入。

首次发布前，维护者必须把 `host-source.json` 固定为包含 API 2、项目图库解析器及其工具的可信 Mewu **完整提交 SHA**。`commit:null` 会让 CI 明确失败，不能用 `next`、`main` 或作者 PR 中的代码代替。审核流程只检出社区可信 base，构建固定宿主源码中的原生校验器，随后通过固定 Git tree/blob API 把 PR 的条目和项目元数据读取为 JSON 数据；不检出、不加载、不执行作者的脚本、ZIP 源码或工作流。GitHub 令牌只用于读取官方 API，PR 任务无写权限。合并后主分支任务重新核验真实发行、独立源码和截图，只提交新版安装、兼容安装与图库三个生成索引。

```powershell
node --test tools/extension-market/test.mjs tools/extension-market/project-test.mjs
node --test tools/extension-market/native-test.mjs tools/extension-market/project-native-test.mjs
```

第一套是 ZIP、网络边界、进程包装、原子文件、PR 数据读取测试，使用明确的传输模拟；不声称验证真实宿主。第二套必须显式提供最新版原生校验器，缺少时直接失败，没有 mock 或静默 skip。线上真实发行下载和插件运行另行验收。

实现依据：[GitHub Release Asset API](https://docs.github.com/en/rest/releases/assets)、[Git blob API](https://docs.github.com/en/rest/git/blobs)、[GitHub Actions 不可信检出防护](https://docs.github.com/en/actions/reference/security/secure-use#mitigating-the-risks-of-untrusted-code-checkout)、[DSH 社区市场](https://github.com/dsh-market/dsh-market)。Mewu 借鉴独立仓库发行、目录 PR 收录与可信 CI，不兼容 DSH 的 npm 执行模型。
