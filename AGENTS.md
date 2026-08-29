# 喵呜AI 开发约束
- Windows-only，保持 C#、WPF 与现代 .NET，不迁移到其他 UI 技术。
- Capture 与 Presentation 分层；离线基础能力不得依赖 AI。
- 核心区域使用物理像素，集中处理多屏、负坐标和 DPI。
- AI Provider 与 UI 解耦；用户明确发送前不得上传屏幕内容。
- 不提交密钥、`.env`、用户配置、日志、临时截图或录屏。
- 禁止 GPL/AGPL；优先微软官方、MIT、BSD、Apache-2.0。
- 重大修改必须构建和测试，仓库始终保持可运行、可回滚。
- 区域 MP4 使用 MIT 的 ScreenRecorderLib + Windows Media Foundation；不要改为捆绑 GPL FFmpeg。
