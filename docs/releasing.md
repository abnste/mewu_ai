# 桌面发行

Windows、macOS、Linux 使用同一提交和版本。源码回归、构建成功不代表原生界面和安装验收通过。三个平台均完成验收后，统筹者才运行发布命令。CI 只生成候选包，没有发布权限，也不会因为推送标签自动发布。

当前打包目标为 Windows x64 的 NSIS 安装包与便携 ZIP、macOS arm64 的 DMG、Linux x64 的 AppImage。平台支持范围和系统最低要求应以实际测试结果及对应版本说明为准，不由这些打包目标推断。

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
node scripts/assemble-release.mjs /absolute/private/assembly-plan.json
node scripts/publish-release.mjs out/release
```

发布前再次核验三端回执、提交、版本与成品字节。已发布的版本、同版本安装包和更新归档不替换；仅固定 JSON 更新入口可以推进。许可保持 MPL-2.0，安装包内保留第三方许可。

自动更新使用固定 `update-channel` 发行中的 JSON，不依赖分支名字。Windows 与 Linux 更新直接使用版本发行的安装包；Mac 更新使用独立的 `.app.tar.gz`，放在更新通道中，版本下载页只提供 DMG。更新签名嵌入 JSON；不发布独立 `.sig`、密钥、开发文档或测试附件。macOS 更新包签名不等同于 Developer ID 签名或公证，必须分别验收。

数据库升级前正常退出程序、保存完整冷备；数据库版本提高后，旧程序不能直接打开新库。回滚须使用匹配的旧程序及旧数据备份，不能降版本号或覆盖升级后新增内容。
