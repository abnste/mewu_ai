# 0.7.4 升级到预览版

Windows 0.7.4 用户可以在「设置 → 关于」检查更新，确认下载后安装预览版 4。安装后的软件版本为 `1.0.0-preview.4`，后续使用现有预览更新通道。

旧更新器只读取 GitHub 的 latest Release，拒绝预览标记以及含 `-preview` 的标签。因此，旧版更新入口使用数值兼容标签 [`v1.0.0.4`](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0.4)，附件名为 `MewuAI-Remake-Setup-1.0.0.4-win-x64.exe`。它与[预览版 4](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0-preview.4) 的 Windows 安装包逐字节相同；`1.0.0.4` 是旧更新协议的兼容标识，不是另一套产品版本。

更新器先校验 GitHub 提供的安装包 SHA-256，再以 NSIS 的 `/S` 参数启动安装。旧程序随后正常退出，安装完成后可从快捷方式启动重制版。安装器会沿用已登记的旧版每用户安装目录；以管理员身份安装的旧版不在这一目录迁移范围内。旧版数据文件保留；支持的 API 连接会一次导入。旧版历史对话尚未转换为新版会话，部分旧配置也不支持自动迁移。请保留原数据备份。

0.7.3 及更早版本应先从[0.7.4 发行页](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4) 安装 0.7.4，然后检查更新。此兼容入口只服务旧版 Windows 更新器；Mac、Linux 和新版 Windows 的正常下载仍在预览版发行页。旧更新器使用系统网络代理；GitHub REST 限流时可能只能提示版本，需稍后重试或从发行页手动下载。

## 维护规则

- 兼容入口只复用已发布、三端验收完成的预览版 Windows 安装包；必须校验源标签、原始附件大小和 SHA-256，再上传相同字节的兼容附件。
- 原预览版的标签、附件、预览属性和固定更新 JSON 保持原样。兼容入口不能替代常规 SemVer 发行流程，也不能当作新编译的软件版本。
- 兼容发行正文必须说明实际安装版本及对应源码。已公开的兼容附件不可覆盖；中断时只恢复同来源、同字节的草稿。
- GitHub latest 指向兼容入口，才能被现有 0.7.4 发现。将来正式发布稳定 1.x 时，明确将 latest 切换到该正式发行。

## English

Windows 0.7.4 can upgrade through Settings → About. The compatibility release `v1.0.0.4` supplies the unchanged preview 4 Windows installer using the filename required by the old updater. The installed version remains `1.0.0-preview.4` and subsequently follows the fixed preview channel. The existing preview release and its packages remain unchanged.

Supported API connections are imported once. Legacy files remain available, but old conversation history and all preferences are not automatically converted. Versions before 0.7.4 should first install 0.7.4 from its versioned release page.
