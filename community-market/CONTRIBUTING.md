# 收录插件

1. 在自己的公开 GitHub 仓库开发 API 2 插件。清单为包根 `mewu-plugin.json`，`schemaVersion:2`、`hostApi:2`、`runtime:"javascript"`；ES Modules 通过 Mewu SDK 的 `activate` 注册服务。完整开发契约和工具见 [宿主插件工具](https://github.com/abnste/mewu_ai/tree/next/tools/extension-market)。
2. 审查插件实际行为和权限。没有必要的权限不要申请；不得夹带密钥、用户数据、原生程序、安装脚本或隐瞒网络/屏幕/输入行为。公开仓库包含许可和可供审阅的源代码。
3. 使用宿主的 `pack` 工具产生没有外层目录的 ZIP，在自己的仓库创建版本 release，用 `publish` 上传并重新下载核验，产生 `<plugin-id>.json`。不要手写零摘要、示例仓库或不支持的平台。
4. 向本仓库提交 `entries/<plugin-id>.json`，PR 仅改 entries 内最多三个条目。升级保留插件 ID 和原 GitHub 仓库、增加新版本/发行 tag，使用真实的新 ZIP SHA256 与体积。维护源码、流程或文档的 PR 单独提交。
5. 在 PR 中说明用途、仓库和发行地址、权限用途、支持的平台、实际测试方式与结果。维护者会阅读代码并检查描述；自动下载校验不代表插件安全或功能已经实测。

目录条目字段由宿主的同一个严格 Rust 解析器核验，不兼容 API 1 声明清单。条目必须与实际 ZIP 内清单的 `id/name/description/version/tags/hostApi` 一致；GitHub `repository` 必须是 `owner/repo`，固定 `releaseTag`、`.zip` 的 `assetName`、完整 `sha256`、正整数 `size`。`platforms` 为 `["any"]` 或已验证平台列表；区域输入能力尚未提供的平台不能宣称兼容。

不要在 PR 中修改 `mewu-catalog.json`。CI 从可信基础分支和固定宿主源码运行校验工具，按固定 Git tree/blob 读取 PR 数据，不执行作者代码。合并后重新核验实际 release 并自动提交生成索引。资产被删除或替换、摘要/体积/身份不符、目录换源、未知字段都会失败，旧公开索引保留。

插件 ID 的所有权不因临时撤下而开放抢注。仓库转移、改名、接管及安全撤下由维护者单独处理并审查历史。通常不接受同一 ID 换到另一仓库；作者希望转移时不要仅改 JSON 绕过检查。

维护者初始化本仓库时，必须先将 `host-source.json` 中 `commit` 设为已上传、包含 API 2 实现的可信完整源码 SHA，再启用收录。禁止以浮动分支或 PR 提供的解析器代替。
