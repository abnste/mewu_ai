# 桌面发行

Windows、macOS、Linux 使用同一提交和版本。源码回归、构建成功不代表原生界面和安装验收通过。三个平台均完成验收后，统筹者才运行发布命令。CI 只生成候选包，没有发布权限，也不会因为推送标签自动发布。

当前打包目标为 Windows x64 的 NSIS 安装包与便携 ZIP、macOS arm64 的 DMG、Linux x64 的 AppImage。平台支持范围和系统最低要求应以实际测试结果及对应版本说明为准，不由这些打包目标推断。

默认分支 `master` 用于重制版开发；原 .NET/WPF 代码保留在 `dotnet`。分支改名不改已发布的标签、安装包或固定更新通道。

首个正式版本使用 `1.0.0`、标签 `v1.0.0`，说明见 [1.0.0 发行说明](release-notes-1.0.0.md)。正式构建的更新端点必须为固定 `update-channel/latest.json`；现有预览客户端保留 `preview.json`。正式发行会同时推进这两个入口，让预览客户端升级到正式版；后续预览发行只推进 `preview.json`，不让正式客户端自动收到预览版。

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

发布前再次核验三端回执、提交、版本与成品字节。已发布的版本、同版本安装包和更新归档不替换；仅固定 JSON 更新入口可以推进。许可保持 MPL-2.0，安装包内保留第三方许可。

版本下载页仅上传这四项（`<version>` 为 `1.0.0`，预览发行则使用包含预览后缀的完整版本）：

- `MewuAI-Remake-Setup-<version>-win-x64.exe`
- `MewuAI-Remake-Portable-<version>-win-x64.zip`
- `MewuAI-Remake-<version>-mac-arm64.dmg`
- `MewuAI-Remake-<version>-linux-x64.AppImage`

发布脚本先建立或恢复同版本草稿、校验固定源提交并上传成品，再公开版本，最后推进更新 JSON。草稿上传中断后用原提交与原始字节重跑；源提交或已上传内容不同则拒绝，不删除旧资产来强行重试。不要额外上传测试回执、校验日志、SDK、插件包或开发目录；社区插件有自己的发行仓库。

自动更新使用固定 `update-channel` 发行中的 JSON，不依赖分支名字。Windows 与 Linux 更新直接使用版本发行的安装包；Mac 更新使用独立的 `.app.tar.gz`，放在更新通道中，版本下载页只提供 DMG。更新签名嵌入 JSON；不发布独立 `.sig`、密钥、开发文档或测试附件。macOS 更新包签名不等同于 Developer ID 签名或公证，必须分别验收。

正式发行须设置 `prerelease: false` 并显式成为 GitHub latest。发布后匿名核对 `v1.0.0` 标签、四项公开下载、Mac 更新归档及两个固定 JSON；两个通道应指向本轮相同版本、相同平台字节与版本绑定签名。旧预览标签和安装包不覆盖、不改名。检查预览客户端能发现正式版，正式客户端使用 `latest.json`，以及 0.7.4 的 latest 入口能识别正式安装包。

数据库升级前正常退出程序、保存完整冷备；数据库版本提高后，旧程序不能直接打开新库。回滚须使用匹配的旧程序及旧数据备份，不能降版本号或覆盖升级后新增内容。

## 旧版 Windows 更新入口

0.7.4 的已安装更新器只接受数值标签和 GitHub latest 正式入口。正式 `v1.0.0` 符合该协议，使用标准 `MewuAI-Remake-Setup-1.0.0-win-x64.exe`，无需另建兼容别名。正式发布时显式将 latest 从历史 `v1.0.0.4` 切到 `v1.0.0`，不依赖 GitHub 自动排序。

历史 `v1.0.0.4` 用于用户选择升级到预览版 4，实际安装 `1.0.0-preview.4`；它不代表另一套正式产品。以下规则仅适用于这类历史协议兼容入口。详见[旧版升级说明](legacy-updates.md)。

这是既有安装包的协议兼容入口：先校验已发布预览版的固定源提交及安装包实际字节，再以旧更新器要求的附件名上传相同安装包。它不构建新产品，不修改原预览版，不推进固定更新 JSON，也不免除源预览版的三端验收。兼容发行只附一个 Windows 安装包，不上传签名、测试文件或开发资料；正文明确实际版本与来源，避免把数值标签误认为新产品版本。

兼容入口的私有计划使用 `schemaVersion: 2`，明确 `workingBranch: "master"`、`protectedBranch: "dotnet"` 和旧分支的完整 `expectedLegacyCommit`。发布前及上传后均要求本地 `master` 干净、HEAD 与 `origin/master` 等于审阅过的 `targetCommit`，并核对本地跟踪及 GitHub `dotnet` 的固定提交；旧 schema 1 计划拒绝执行，保留为历史证据。当前 `dotnet` 保护基线为 `801511c17760c2595c9418a21acc8f31a9e2dcfe`。已发布的 `v1.0.0.4` 仍绑定原文档与脚本提交，不随分支改名重发或重绑。
