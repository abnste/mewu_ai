# Mewu 插件市场

这个仓库收录 Mewu API 2 JavaScript 插件的真实 GitHub 发行。插件代码和版本包由作者在自己的仓库维护；本仓库保存每插件一个目录条目，经审核及 ZIP 下载核验后生成公开索引。

向市场贡献插件请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。Mewu 宿主、SDK、打包和发行工具在 [abnste/mewu_ai](https://github.com/abnste/mewu_ai/tree/next)，无需向宿主核心提交 PR 才能新增普通插件逻辑。

- `entries/<plugin-id>.json`：社区提交的发行记录。
- `mewu-catalog.json`：Mewu 实际读取的 API 2 索引。
- `host-source.json`：维护者固定的可信宿主解析器源码完整 SHA。
- `.github/workflows`：只把作者 PR 当数据的核验及合并后索引生成。

初始化目录可以为空。没有验证的示例、内置功能或占位仓库不会冒充社区插件。

本仓库采用 MPL-2.0；各插件遵循其自身声明的许可。
