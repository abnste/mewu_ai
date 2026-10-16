# 旧版 Windows 升级

## 升级到 1.0.0 正式版

Windows 0.7.4 用户可在「设置 → 关于」检查更新，确认下载后安装重制版。GitHub latest 指向[当前正式发行](https://github.com/abnste/mewu_ai/releases/latest)，旧更新器可直接使用正式 Windows 安装包，无需新的数值兼容标签。修订后的 1.0.0 从最新正式发行读取更新索引和安装包，不再依赖 `update-channel`。

早期预览版仍可通过原入口升级到修订后的 1.0.0。已经安装早期 1.0.0 的用户需要从正式发行页重新下载安装一次，因为相同版本号不会触发自动升级。安装后使用新的正式更新地址。旧入口仅为未迁移客户端保留，不能用它判断新正式版仍有依赖。

更新器先校验 GitHub 提供的安装包 SHA-256，再以 NSIS 的 `/S` 参数启动安装。旧程序随后正常退出，安装完成后可从原快捷方式启动重制版。安装器会沿用已登记的旧版每用户安装目录；以管理员身份安装的旧版不在这一目录迁移范围内。

支持的旧 Chat Completions、OpenAI Responses 和 Anthropic Messages API 连接会一次导入，包含对应模型和本机加密密钥；已修改的新连接不会被反复覆盖。自定义请求头、非系统代理和部分路由配置不自动导入，需要在新版重新配置。**原数据文件保留，但旧历史对话尚未转换到新版历史界面，偏好设置和集成并非全部迁移。** 请备份重要数据，具体范围见 [1.0.0 说明](release-notes-1.0.0.md)。

0.7.3 及更早版本应先从[0.7.4 发行页](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4) 安装 0.7.4，然后检查更新。正常下载见 [1.0.0 发行页](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0)。旧更新器使用系统网络代理；GitHub REST 限流时可能只能提示版本，需稍后重试或从发行页手动下载。

## 预览版 4 的历史兼容入口

旧更新器只读取 GitHub 的 latest Release，拒绝预览标记以及含 `-preview` 的标签。因此，当时使用数值兼容标签 [`v1.0.0.4`](https://github.com/abnste/mewu_ai/tree/v1.0.0.4)，附件名为 `MewuAI-Remake-Setup-1.0.0.4-win-x64.exe`。它与[预览版 4](https://github.com/abnste/mewu_ai/tree/v1.0.0-preview.4) 的 Windows 安装包逐字节相同；`1.0.0.4` 是旧更新协议的兼容标识，实际产品版本为 `1.0.0-preview.4`。

1.0.0 正式版接管更新后，预览版和临时兼容发行及安装包已清理；对应源码标签保留。正式版使用自己的 `v1.0.0` 标签与安装包，不能拿旧预览包改名当正式版。

## 维护规则

- 重制版在默认分支 `master` 开发，旧 .NET/WPF 代码保留在 `dotnet`。私有发行计划使用 schema 2，明确 `workingBranch: "master"`、`protectedBranch: "dotnet"`、完整 `expectedLegacyCommit` 与 `targetCommit`；旧 schema 1 计划只作历史记录，不能再执行。发布器同时核对干净工作树、HEAD、`origin/master` 以及本地跟踪和 GitHub 上的旧分支提交。
- 兼容入口只复用已发布、三端验收完成的预览版 Windows 安装包；必须校验源标签、原始附件大小和 SHA-256，再上传相同字节的兼容附件。
- 历史兼容入口不修改源预览版标签、附件、预览属性或更新 JSON。后续正式 SemVer 发行可按正常流程推进固定 JSON；兼容入口本身不能替代常规发行流程或当作新编译的软件版本。
- 兼容发行正文必须说明实际安装版本及对应源码。已公开的兼容附件不可覆盖；中断时只恢复同来源、同字节的草稿。
- 正式 1.0.0 发布前，GitHub latest 曾由历史兼容入口提供。修订后的正式版使用版本发行自带的索引；原 `preview.json`、`latest.json` 和旧 Mac TAR 地址仅供旧客户端兼容，不属于新的正式发布流程。

## English

Windows 0.7.4 discovers the current stable GitHub release through Settings → About and installs its regular Windows package. Earlier 0.7.x versions should first install 0.7.4. Revised 1.0.0 builds use the manifest and updater archive in versioned stable releases, independently of update-channel. Earlier previews can upgrade through their original endpoint; earlier 1.0.0 installations need one manual reinstall because the repair keeps the same version.

Supported API connections and credentials are imported once. Legacy files remain, but old conversations are not converted into the new history UI; preferences and integrations are not fully migrated. Custom headers, non-system proxy settings and some routing configurations need manual setup. Keep a backup of important data.

The historical `v1.0.0.4` compatibility release supplied the unchanged preview 4 installer, whose product version was `1.0.0-preview.4`. Preview and temporary compatibility releases and their packages were removed after stable 1.0.0 took over. Source tags and the fixed update channel remain.
