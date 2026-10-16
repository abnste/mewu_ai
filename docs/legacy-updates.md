# 旧版 Windows 升级

## 升级到 1.0.0 正式版

Windows 0.7.4 用户可在「设置 → 关于」检查更新，确认下载后安装重制版。1.0.0 正式发行后，GitHub latest 指向 [`v1.0.0`](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0)，旧更新器可直接使用正式 Windows 安装包，无需新的数值兼容标签。安装后的正式版通过固定正式通道继续更新；已有预览版通过原预览通道同样可升级到正式版。

更新器先校验 GitHub 提供的安装包 SHA-256，再以 NSIS 的 `/S` 参数启动安装。旧程序随后正常退出，安装完成后可从原快捷方式启动重制版。安装器会沿用已登记的旧版每用户安装目录；以管理员身份安装的旧版不在这一目录迁移范围内。

支持的旧 Chat Completions、OpenAI Responses 和 Anthropic Messages API 连接会一次导入，包含对应模型和本机加密密钥；已修改的新连接不会被反复覆盖。自定义请求头、非系统代理和部分路由配置不自动导入，需要在新版重新配置。**原数据文件保留，但旧历史对话尚未转换到新版历史界面，偏好设置和集成并非全部迁移。** 请备份重要数据，具体范围见 [1.0.0 说明](release-notes-1.0.0.md)。

0.7.3 及更早版本应先从[0.7.4 发行页](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4) 安装 0.7.4，然后检查更新。正常下载见 [1.0.0 发行页](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0)。旧更新器使用系统网络代理；GitHub REST 限流时可能只能提示版本，需稍后重试或从发行页手动下载。

## 预览版 4 的历史兼容入口

旧更新器只读取 GitHub 的 latest Release，拒绝预览标记以及含 `-preview` 的标签。因此，当时使用数值兼容标签 [`v1.0.0.4`](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0.4)，附件名为 `MewuAI-Remake-Setup-1.0.0.4-win-x64.exe`。它与[预览版 4](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0-preview.4) 的 Windows 安装包逐字节相同；`1.0.0.4` 是旧更新协议的兼容标识，实际产品版本为 `1.0.0-preview.4`。

这一历史标签、附件和来源继续保留，不重命名、覆盖或重新绑定。正式版使用自己的 `v1.0.0` 标签与安装包，不能拿旧预览包改名当正式版。

## 维护规则

- 重制版在默认分支 `master` 开发，旧 .NET/WPF 代码保留在 `dotnet`。私有发行计划使用 schema 2，明确 `workingBranch: "master"`、`protectedBranch: "dotnet"`、完整 `expectedLegacyCommit` 与 `targetCommit`；旧 schema 1 计划只作历史记录，不能再执行。发布器同时核对干净工作树、HEAD、`origin/master` 以及本地跟踪和 GitHub 上的旧分支提交。
- 兼容入口只复用已发布、三端验收完成的预览版 Windows 安装包；必须校验源标签、原始附件大小和 SHA-256，再上传相同字节的兼容附件。
- 历史兼容入口不修改源预览版标签、附件、预览属性或更新 JSON。后续正式 SemVer 发行可按正常流程推进固定 JSON；兼容入口本身不能替代常规发行流程或当作新编译的软件版本。
- 兼容发行正文必须说明实际安装版本及对应源码。已公开的兼容附件不可覆盖；中断时只恢复同来源、同字节的草稿。
- 正式 1.0.0 发布前，GitHub latest 仍可由历史兼容入口服务。正式发布时必须显式切换 latest 到 `v1.0.0`，同时推进 `preview.json` 和 `latest.json`；不要依赖 GitHub 对 `v1.0.0.4` 与 `v1.0.0` 的自动排序。

## English

Once stable 1.0.0 is published as GitHub latest, Windows 0.7.4 can discover it through Settings → About and install the regular 1.0.0 Windows package. Earlier 0.7.x versions should first install 0.7.4. Stable builds follow the fixed stable feed; existing previews receive the stable release through their original preview feed.

Supported API connections and credentials are imported once. Legacy files remain, but old conversations are not converted into the new history UI; preferences and integrations are not fully migrated. Custom headers, non-system proxy settings and some routing configurations need manual setup. Keep a backup of important data.

The historical `v1.0.0.4` compatibility release retains its unchanged preview 4 installer, whose installed product version is `1.0.0-preview.4`. Its original tag and payload are not replaced by the stable release.
