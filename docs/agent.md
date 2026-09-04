# 发布文档与配图

- README 英文和中文分别使用自然语言介绍同一组功能，发行日志正文不重复 GitHub Release 标题。
- v0.2.0 的五张 JPG 及视频由用户在发布请求中明确指定并授权公开。除此以外不得上传实际桌面、提示词、设置、日志或 QA 产物；`patent-research-2026-09-02.md` 为用户本地文件，禁止加入发布提交。
- `docs/images/ai-checkmarks.jpg`、`in-place-translation.jpg`、`ai-drawing.jpg`、`code-explanation.jpg`、`web-annotations.jpg` 保留用户提供的原图，双语说明描述能力而非保证模型正确。
- 用户视频原编码为 HEVC；`docs/media/MewuAI-video-annotations.mp4` 转为 H.264/yuv420p 并保留原 AAC 音轨，以便普通浏览器和 Windows 播放。转换工具只用于本地文档处理，不进入产品依赖或安装包。
- Release 将演示 MP4 单独上传；应用 EXE/ZIP 仍不得携带文档图片和视频。更新器的 SHA256SUMS 只覆盖两个版本化安装/便携资产，不把演示文件当更新包。
- Release 配图链接绑定版本标签，不使用会随 master 改变的图片地址；README、安装包、csproj 与发布说明必须版本一致。
