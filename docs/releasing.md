# 桌面发行

Windows、macOS、Linux 使用同一提交和版本。源码回归、构建成功不代表原生界面和安装验收通过。三个平台均完成验收后，统筹者才运行发布命令。CI 只生成候选包，没有发布权限，也不会因为推送标签自动发布。

当前打包目标为 Windows x64 的 NSIS 安装包与便携 ZIP、macOS arm64 的 DMG、Linux x64 的 AppImage。平台支持范围和系统最低要求应以实际测试结果及对应版本说明为准，不由这些打包目标推断。

先在 `next` 完成版本号、锁文件和 `docs/release-notes-<version>.md`，提交后冻结完整 SHA；三个平台均从该提交构建，不用旧包重命名。手动触发 `.github/workflows/preview-release.yml`，保留各平台的候选产物和源码测试结果，再对这些最终字节完成原生安装验收。该工作流不会签名或发布安装包；协调者只对验收完成的更新产物生成更新签名。

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
node scripts/assemble-release.mjs .private/release/assembly-plan.json
node scripts/publish-release.mjs out/release
```

发布前再次核验三端回执、提交、版本与成品字节。已发布的版本、同版本安装包和更新归档不替换；仅固定 JSON 更新入口可以推进。许可保持 MPL-2.0，安装包内保留第三方许可。

版本下载页仅上传这四项（`<version>` 包含 `1.0.0-preview.4` 这样的完整版本）：

- `MewuAI-Remake-Setup-<version>-win-x64.exe`
- `MewuAI-Remake-Portable-<version>-win-x64.zip`
- `MewuAI-Remake-<version>-mac-arm64.dmg`
- `MewuAI-Remake-<version>-linux-x64.AppImage`

发布脚本先建立或恢复同版本草稿、校验固定源提交并上传成品，再公开版本，最后推进更新 JSON。草稿上传中断后用原提交与原始字节重跑；源提交或已上传内容不同则拒绝，不删除旧资产来强行重试。不要额外上传测试回执、校验日志、SDK、插件包或开发目录；社区插件有自己的发行仓库。

自动更新使用固定 `update-channel` 发行中的 JSON，不依赖分支名字。Windows 与 Linux 更新直接使用版本发行的安装包；Mac 更新使用独立的 `.app.tar.gz`，放在更新通道中，版本下载页只提供 DMG。更新签名嵌入 JSON；不发布独立 `.sig`、密钥、开发文档或测试附件。macOS 更新包签名不等同于 Developer ID 签名或公证，必须分别验收。

数据库升级前正常退出程序、保存完整冷备；数据库版本提高后，旧程序不能直接打开新库。回滚须使用匹配的旧程序及旧数据备份，不能降版本号或覆盖升级后新增内容。

## 旧版 Windows 更新入口

0.7.4 的已安装更新器只接受数值标签和 GitHub latest 正式入口。为支持用户选择升级到预览版，可建立单独的数值兼容发行；当前 `v1.0.0.4` 实际安装 `1.0.0-preview.4`。详见[旧版升级说明](legacy-updates.md)。

这是既有安装包的协议兼容入口：先校验已发布预览版的固定源提交及安装包实际字节，再以旧更新器要求的附件名上传相同安装包。它不构建新产品，不修改原预览版，不推进固定更新 JSON，也不免除源预览版的三端验收。兼容发行只附一个 Windows 安装包，不上传签名、测试文件或开发资料；正文明确实际版本与来源，避免把数值标签误认为新产品版本。
