# JavaScript 插件发行与社区市场

Mewu API 2 插件在独立 QuickJS 进程中执行 ES Modules，通过 SDK 动态注册命令、工具、Agent、记忆、模型、渠道、渲染器和界面服务。这个工具负责把独立插件打成 ZIP、发布到 GitHub Release，再核验社区目录中的真实发行资产。它不启动插件、不调用模型、不执行作者安装脚本。

需要 Node.js 22+、GitHub CLI（仅 `publish` 需要），以及从可信 Mewu 源码构建的独立校验器。插件清单、目录条目、完整目录均把**原始 JSON 字节**交给宿主的同一 Rust parser；没有另一份 JavaScript 清单 schema。API 1 的声明插件工具仍在相邻 `plugin-catalog` 中，不能代替 API 2 校验器。

```powershell
cargo build -p mewu-desktop --bin mewu-plugin-validator --features catalog-validator --locked
$env:MEWU_PLUGIN_VALIDATOR = (Resolve-Path target/debug/mewu-plugin-validator.exe).Path
node tools/extension-market/cli.mjs --help
```

API 2 校验器额外返回 `extensionContractVersion: 2` 和 `extensionContractSha256`；指纹按照 `contract.rs`、`package.rs`、`sdk.js`、`extension_market.rs` 的原始字节，以 NUL 分隔计算。源码变化后必须重编译，不能用旧二进制冒充当前格式验收。指纹检测契约失配，不证明一个外来 EXE 的来源。

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

`build` 默认实际核验每条 GitHub Release ZIP。`--offline` 仅做开发结构检查，结果明确标识 `releasesVerified:false`；公开目录发布不可使用它。`--previous` 指定可信旧目录，避免 PR 通过替换生成文件掩盖 ID 换源；已有 ID 的仓库不能悄悄变更。安装端仍要独立绑定已安装插件的来源并重新验包。对于移除后保留的永久 ID 所有权，目录维护者须保留历史或保留条目，不能只凭新 PR 的声明认可转移。

条目目录仅接受直属 `.json` 普通文件，最多 256 条，总计和生成索引各不超过 2 MiB；目录、软链接、杂项和重复身份会失败。全部格式、来源、网络、摘要核验通过后才在输出同目录写临时文件、同步、rename 替换；失败保留原索引。`--check` 不写文件。输出不能放在输入 entries 中。标准 GitHub 下载仅从 API 开始，人工处理 HTTPS 重定向，只允许 GitHub 的资产域名，认证头不随下载重定向转发。

## 社区仓库与 CI

[community-market](../../community-market/README.md) 可作为独立 `abnste/mewu-plugins` 仓库发布：`entries/<id>.json` 供 PR 收录，`mewu-catalog.json` 是宿主实际读取的生成索引。初始化模板的索引为空，只有真实发布并验证过的资产才可加入。

首次发布前，维护者必须把 `host-source.json` 固定为包含 API 2 解析器的可信 Mewu **完整提交 SHA**。`commit:null` 会让 CI 明确失败，不能用 `next`、`main` 或作者 PR 中的代码代替。审核流程只检出社区可信 base，构建固定宿主源码中的原生校验器，随后通过固定 Git tree/blob API 把 PR 条目读取为 JSON 数据；不检出、不加载、不执行作者的脚本、ZIP 源码或工作流。GitHub 令牌只用于读取官方 API，PR 任务无写权限。合并后主分支任务重新核验所有真实资产并只提交索引。

```powershell
node --test tools/extension-market/test.mjs
node --test tools/extension-market/native-test.mjs
```

第一套是 ZIP、网络边界、进程包装、原子文件、PR 数据读取测试，使用明确的传输模拟；不声称验证真实宿主。第二套必须显式提供最新版原生校验器，缺少时直接失败，没有 mock 或静默 skip。线上真实发行下载和插件运行另行验收。

实现依据：[GitHub Release Asset API](https://docs.github.com/en/rest/releases/assets)、[Git blob API](https://docs.github.com/en/rest/git/blobs)、[GitHub Actions 不可信检出防护](https://docs.github.com/en/actions/reference/security/secure-use#mitigating-the-risks-of-untrusted-code-checkout)、[DSH 社区市场](https://github.com/dsh-market/dsh-market)。Mewu 借鉴独立仓库发行、目录 PR 收录与可信 CI，不兼容 DSH 的 npm 执行模型。
