# 发布文档与配图

- 功能实景图与视频介绍只放在英文和中文 README，分别使用自然语言介绍同一组功能；发行日志只列版本更新，不重复功能图库或 GitHub Release 标题。
- v0.2.0 的五张 JPG 及视频由用户在发布请求中明确指定并授权公开。除此以外不得上传实际桌面、提示词、设置、日志或 QA 产物；`patent-research-2026-09-02.md` 为用户本地文件，禁止加入发布提交。
- `docs/images/ai-checkmarks.jpg`、`in-place-translation.jpg`、`ai-drawing.jpg`、`code-explanation.jpg`、`web-annotations.jpg` 保留用户提供的原图，双语说明描述能力而非保证模型正确。
- 用户视频原编码为 HEVC；`docs/media/MewuAI-video-annotations.mp4` 转为 H.264/yuv420p 并保留原 AAC 音轨，以便普通浏览器和 Windows 播放。转换工具只用于本地文档处理，不进入产品依赖或安装包。
- Release 将演示 MP4 单独上传；应用 EXE/ZIP 仍不得携带文档图片和视频。更新器的 SHA256SUMS 只覆盖两个版本化安装/便携资产，不把演示文件当更新包。
- README 配图使用仓库内相对路径，演示视频链接绑定已发布的版本资产；README、安装包、csproj 与发布说明必须版本一致。
- 自包含发布前必须单独按 `Release`、`x64`、`win-x64`、`SelfContained=true` 执行锁定还原，再以同样参数 `publish --no-restore`。普通解决方案还原不会保证下载 .NET/WPF Runtime pack；本机已有缓存会掩盖此问题，需用全新包目录验证。v0.2.0 在 CI 因 NETSDK1112 失败，保留原标签不移动，正式包改由 v0.2.1 发布。
- NuGet 自定义包目录不保证 `NuGetPackageRoot` 带结尾分隔符，许可证路径必须显式加入目录分隔符；全新缓存的发布演练已覆盖该差异，不能只依赖默认用户缓存。
- `workflow_dispatch` 用于新标签前的完整云端预检：编译、测试、publish 审计和安装包打包全部照常，只跳过 GitHub Release 创建。v0.2.1 被真实录屏取消测试拦住，保留失败标签，后续发布用 v0.2.2。
