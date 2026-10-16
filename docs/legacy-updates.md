# 旧版 Windows 升级

## 升级到当前正式版

从 [1.0.1 正式发行页](https://github.com/abnste/mewu_ai/releases/tag/v1.0.1) 下载 Windows 安装包，正常退出旧程序后直接安装，无需卸载。安装完成后可使用原快捷方式启动 MewuAI。安装器沿用已登记的旧版每用户安装目录；以管理员身份安装的旧版不在这一目录迁移范围内。

已安装的 0.7.4、预览版和 1.0.0 可能写死旧附件名或更新清单地址，需要手动安装这一次。1.0.1 使用统一的 MewuAI 包名；之后在「设置 → 关于」直接查询 GitHub 正式发行完成更新，验证原公钥及签名绑定版本，不再依赖额外清单或 `update-channel`。旧程序不能因为服务器更新就自动识别新的协议与名称。

支持的旧 Chat Completions、OpenAI Responses 和 Anthropic Messages API 连接会一次导入，包含对应模型和本机加密密钥；已修改的新连接不会被反复覆盖。自定义请求头、非系统代理和部分路由配置不自动导入，需要在新版重新配置。**原数据文件保留，但旧历史对话尚未转换到新版历史界面，偏好设置和集成并非全部迁移。** 请备份重要数据，具体范围见 [1.0.0 说明](release-notes-1.0.0.md)。

数据库升级后不能直接使用旧程序打开新库。回滚须使用匹配的旧程序及升级前数据备份，不能覆盖升级后新增的内容。

## 历史更新协议

Windows 0.7.4 的更新器只读取 GitHub latest，匹配纯数值标签和固定的 `MewuAI-Remake-Setup-<version>-win-x64.exe`。它曾在自身更新界面下载并安装 1.0.0；这个历史流程不等于它能识别 1.0.1 的新包名。旧程序使用系统网络代理，GitHub REST 限流时可能仅提示版本。历史安装包仍见 [0.7.4 发行页](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4)。

预览版 4 曾使用数值兼容标签 [`v1.0.0.4`](https://github.com/abnste/mewu_ai/tree/v1.0.0.4)，附件名 `MewuAI-Remake-Setup-1.0.0.4-win-x64.exe`。它与[预览版 4](https://github.com/abnste/mewu_ai/tree/v1.0.0-preview.4) 的 Windows 安装包逐字节相同，实际产品版本为 `1.0.0-preview.4`。正式 1.0.0 发布后，预览版与临时兼容发行及其安装包已清理，对应源码标签保留。

旧客户端曾使用 `preview.json`、`latest.json` 和 `update-channel` 下的 Mac 归档。1.0.1 不再使用或推进这些入口，也不额外挂旧名重复附件。历史脚本与说明用于核验旧版本，不属于当前发行流程。

## 维护规则

- MewuAI 在默认分支 `master` 开发，旧 .NET/WPF 代码保留在 `dotnet`，保护基线为 `801511c17760c2595c9418a21acc8f31a9e2dcfe`。
- 正式发行使用真实新版本、冻结源码和三端验收完成的安装包；已发布版本及其附件不可覆盖。
- 历史兼容入口只复用当时已验收安装包，不能将预览包改名冒充新的正式产品。源码标签和原始字节证据保留，不随默认分支改名重绑。
- API 配置的导入、原文件保留和旧会话转换是不同事项，不能把保留文件描述为全部旧数据已经迁移到新界面。

## English

Install the Windows package from the [1.0.1 release](https://github.com/abnste/mewu_ai/releases/tag/v1.0.1) after quitting the old application. Uninstalling is unnecessary. Installed 0.7.4, preview and 1.0.0 clients may have fixed old asset names or feed URLs and need this one manual installation. Subsequent updates read GitHub Releases directly and retain package signature and version verification.

Supported API connections and credentials are imported once. Legacy files remain, but old conversations are not converted into the new history UI; preferences and integrations are not fully migrated. Custom headers, non-system proxy settings and some routing configurations need manual setup. Keep a backup of important data.

The historical `v1.0.0.4` compatibility release supplied the unchanged preview 4 installer, whose product version was `1.0.0-preview.4`. Preview and temporary compatibility releases were removed after stable 1.0.0 took over; their source tags remain. Old update endpoints are historical client protocols and are not used or advanced by 1.0.1.
