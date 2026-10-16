# 桌面发行

Windows、macOS、Linux 使用同一提交和版本。源码回归、构建成功不代表原生界面和安装验收通过。三个平台均完成验收后，统筹者才运行发布命令。CI 只生成候选包，没有发布权限，也不会因为推送标签自动发布。

当前打包目标为 Windows x64 的 NSIS 安装包与便携 ZIP、macOS arm64 的 DMG、Linux x64 的 AppImage。平台支持范围和系统最低要求应以实际测试结果及对应版本说明为准，不由这些打包目标推断。

默认分支 `master` 用于 MewuAI 开发；原 .NET/WPF 代码保留在 `dotnet`。分支改名不改已发布的标签、安装包或已有数据。

从 1.0.1 起，正式构建直接查询 `https://api.github.com/repos/abnste/mewu_ai/releases/latest`，从正式发行选择当前系统的安装包。版本、下载地址、大小与可用的摘要来自 GitHub Release；更新签名放在发行正文唯一的 `mewu-update-signatures:v1` 隐藏注释中，继续验证原公钥及签名版本。没有额外更新清单文件，不查询、创建或推进 `update-channel`；端点和包路径均不依赖分支名。

1.0.1 使用新版本和新标签，保留已发布的 1.0.0。旧版客户端可能写死旧包名或更新入口，需要手动安装一次，之后由新版自动更新；不能放宽版本比较器或伪造旧客户端升级结果。

先在 `master` 完成版本号、锁文件和 `docs/release-notes-<version>.md`，提交后冻结完整 SHA；三个平台均从该提交构建，不用旧包重命名。手动触发 `.github/workflows/preview-release.yml` 时选择该冻结提交所在的 `master`，保留各平台的候选产物和源码测试结果，再对这些最终字节完成原生安装验收。该工作流不会签名或发布安装包；协调者只对验收完成的更新产物生成更新签名。

原生构建后运行：

```sh
node scripts/prepare-release.mjs out/release-windows-x86_64 windows-x86_64
node scripts/prepare-release.mjs out/release-darwin-aarch64 darwin-aarch64
node scripts/prepare-release.mjs out/release-linux-x86_64 linux-x86_64
```

每条命令在对应平台执行。可用第四个参数指定私有 JSON 文件，其中 `installer`、`updater`、`binary` 是真实产物的完整路径；路径不进入公开元数据。暂存目录只包含明确列出的成品与 `.metadata/build.json`。暂存不会发布。

每个平台的私有验收回执必须包含 `schemaVersion: 1`、`platform`、`version`、`sourceCommit`、`result: "passed"`、`nativeGuiVerified: true`、`installerVerified: true`；`nativeTests` 和 `frontendTests` 包含实际 `passed` 与 `failed` 数量。`artifacts` 的 `name`、`bytes`、`sha256` 必须匹配最终安装与更新产物。

`checks` 中以下项目都必须为 `"passed"`：`packageIntegrity`、`install`、`launch`、`restart`、`capture`、`drawing`、`objects`、`conversation`、`settings`、`recording`、`ocr`、`copySave`、`update`。未测试、受阻或不支持的功能不能写成通过；忽略的测试不算原生验收。回执记录结论，相关截图、日志、系统信息和失败证据另外私有保存。

统筹者使用私有 JSON 计划，`inputs` 将三个平台标识映射到暂存目录，`receipts` 映射到各自的验收回执文件；`signatures` 可映射到本机生成的更新签名文件，`output` 指定工作区内的新目录。三个映射的键是 `windows-x86_64`、`darwin-aarch64`、`linux-x86_64`。

```sh
node scripts/assemble-release.mjs assembly-plan.json
node scripts/publish-release.mjs out/release
```

发布前再次核验三端回执、提交、版本与成品字节。已发布的版本、同版本安装包和更新归档不替换。许可保持 MPL-2.0，安装包内保留第三方许可。

用户安装软件使用这四项（`<version>` 为完整产品版本）：

- `MewuAI-Setup-<version>-win-x64.exe`
- `MewuAI-Portable-<version>-win-x64.zip`
- `MewuAI-<version>-mac-arm64.dmg`
- `MewuAI-<version>-linux-x64.AppImage`

同一正式发行另有 `MewuAI-Update-<version>-mac-arm64.app.tar.gz`，供 macOS 自动更新使用，无需手动打开。公开附件共五个，不提供 `latest.json` 或独立 `.sig`。内部 `.metadata/update.json` 仅作构建验收，发布器将其中三端签名写入发行正文，不上传该文件。

发布脚本先建立或恢复同版本草稿、校验固定源提交并上传所有成品，再公开版本并确认 GitHub Latest。草稿上传中断后用原提交与原始字节重跑；源提交或已上传内容不同则拒绝，不删除旧资产来强行重试。不要额外上传测试回执、校验日志、SDK、插件包或开发目录；社区插件有自己的发行仓库。

Windows 与 Linux 更新使用版本发行的安装包；Mac 使用同一版本发行的 `.app.tar.gz`，用户安装仍选 DMG。原生端严格校验仓库、版本标签、平台文件名、更新签名及签名中的版本。macOS 更新包签名不等同于 Developer ID 签名或公证，必须分别验收。

正式发行须设置 `prerelease: false` 并显式成为 GitHub latest。发布后匿名核对 Latest API 的真实标签、正文签名块、四个软件包与 Mac 更新归档；五个附件的实际字节及三端签名必须与验收一致。正常发布在旧更新通道不存在时也必须成功。

旧预览版及 1.0.0 的更新地址已编译进程序，0.7.4 也会匹配旧包名。1.0.1 不为这些旧协议额外挂重复附件或更新清单，用户需手动安装一次，保留原配置和文件；不能因此宣称旧程序会自动找到新名称。

历史预览和临时兼容发行清理时保留源码标签。旧 `update-channel` 属于历史客户端协议，1.0.1 及后续正式发布不使用或推进它；保留历史说明不代表已经完成客户端迁移。

数据库升级前正常退出程序、保存完整冷备；数据库版本提高后，旧程序不能直接打开新库。回滚须使用匹配的旧程序及旧数据备份，不能降版本号或覆盖升级后新增内容。

## 旧版 Windows 更新入口

0.7.4 的已安装更新器只接受数值标签和 GitHub latest 正式入口。正式 `v1.0.0` 符合该协议，使用标准 `MewuAI-Remake-Setup-1.0.0-win-x64.exe`，无需另建兼容别名。正式发布时显式将 latest 从历史 `v1.0.0.4` 切到 `v1.0.0`，不依赖 GitHub 自动排序。

历史 `v1.0.0.4` 用于用户选择升级到预览版 4，实际安装 `1.0.0-preview.4`；它不代表另一套正式产品。以下规则仅适用于这类历史协议兼容入口。详见[旧版升级说明](legacy-updates.md)。

这是既有安装包的协议兼容入口：先校验已发布预览版的固定源提交及安装包实际字节，再以旧更新器要求的附件名上传相同安装包。它不构建新产品，不修改原预览版，不推进固定更新 JSON，也不免除源预览版的三端验收。兼容发行只附一个 Windows 安装包，不上传签名、测试文件或开发资料；正文明确实际版本与来源，避免把数值标签误认为新产品版本。

兼容入口的私有计划使用 `schemaVersion: 2`，明确 `workingBranch: "master"`、`protectedBranch: "dotnet"` 和旧分支的完整 `expectedLegacyCommit`。发布前及上传后均要求本地 `master` 干净、HEAD 与 `origin/master` 等于审阅过的 `targetCommit`，并核对本地跟踪及 GitHub `dotnet` 的固定提交；旧 schema 1 计划拒绝执行，保留为历史证据。当前 `dotnet` 保护基线为 `801511c17760c2595c9418a21acc8f31a9e2dcfe`。已发布的 `v1.0.0.4` 仍绑定原文档与脚本提交，不随分支改名重发或重绑。
