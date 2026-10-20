# Mewu 插件市场

这个仓库收录 Mewu API 2 JavaScript 插件的真实 GitHub 发行。插件代码和版本包由作者在自己的仓库维护；本仓库保存每插件一个目录条目，经审核及 ZIP 下载核验后生成公开索引。

向市场贡献插件请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。Mewu 宿主、SDK、打包和发行工具在 [abnste/mewu_ai](https://github.com/abnste/mewu_ai/tree/master)，无需向宿主核心提交 PR 才能新增普通插件逻辑。

- `entries/<plugin-id>.json`：社区提交的发行记录。
- `mewu-catalog-v2.json`：新版 Mewu 使用的 API 2 索引，含插件图标。
- `mewu-catalog.json`：旧版 Mewu 的兼容索引，保留原字段。
- `projects/<plugin-id>.json`：独立源码提交和真实截图元数据。
- `mewu-projects.json`：经验证生成的源码与图库索引。
- `host-source.json`：维护者固定的可信宿主解析器源码完整 SHA。
- `.github/workflows`：只把作者 PR 当数据的核验及合并后索引生成。

初始化目录可以为空。没有验证的示例、内置功能或占位仓库不会冒充社区插件。

本仓库采用 MPL-2.0；各插件遵循其自身声明的许可。

## 插件源码

市场仓库只维护收录数据、生成索引和核验流程。官方示例也在各自项目中开发：

- [区域操作 Agent](https://github.com/abnste/mewu-plugin-region-agent)
- [长期记忆](https://github.com/abnste/mewu-plugin-journal-memory)
- [交互组件](https://github.com/abnste/mewu-plugin-interactive-widget)

既有插件发行包和安装来源保持不变；源码归属由固定提交的项目元数据核验。

开发 SDK：[mewu-sdk-0.1.0.tgz](https://github.com/abnste/mewu-plugins/releases/download/v0.1.0/mewu-sdk-0.1.0.tgz)。宿主内置运行时模块；这个包提供开发类型与 ESM SDK，不要求用户安装 Node.js。打包与投稿命令见 [发行工具](https://github.com/abnste/mewu_ai/blob/master/tools/extension-market/README.md)。

## 独立项目与真实图库

两份安装索引均使用 schemaVersion 2，新版 `mewu-catalog-v2.json` 增加图标，原 `mewu-catalog.json` 保留旧版字段；独立 `mewu-projects.json` 绑定插件 ID、版本及安装包 SHA256，提供固定源码提交和真实静态截图。它不改变安装来源或权限。作者提交 `projects/<id>.json`，格式与源码/ZIP/图片验证方式见 [贡献说明](CONTRIBUTING.md)；没有真实截图时保留空数组。

插件图标由 `icon.paths` 声明为 24×24 坐标系的 SVG 路径，可配六位十六进制 `color` 与 `filled`。市场、已安装列表与截图窗口共用图标；完整限制见 [SDK 图标契约](https://github.com/abnste/mewu_ai/blob/master/packages/sdk/README.md)。既有无图标包可以只补目录图标，不替换发行包。
