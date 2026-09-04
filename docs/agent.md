# 发布文档与配图

- 功能实景图与视频介绍只放在英文和中文 README，分别使用自然语言介绍同一组功能；发行日志只列版本更新，不重复功能图库或 GitHub Release 标题。
- README 首屏使用实际标注图，以使用场景串联五张配图和视频入口；构建命令折叠展示。模型仅作为公测说明，不作为产品定位；明确作者、商用需授权及远程 AI 的数据边界。
- README 在非折叠正文明确说明 MiniMax M3 已通过 Hermes 和 API 两种渠道测试，其他模型尚未验证、请用户自行测试兼容性；中英文保持一致，不扩展成所有模型均已验证的承诺。
- README 使用居中品牌与下载区、原位标注大图、两列功能图库及内嵌循环 GIF；保留有声 MP4 链接。GIF 从作者视频转换，保持比例和完整时长，解码验证帧延迟与循环；不加入产品依赖。GitHub About 是单一描述字段，维护简洁中英双语，而非宣称会随浏览器语言切换。
- v0.2.0 的五张 JPG 及视频由用户在发布请求中明确指定并授权公开。除此以外不得上传实际桌面、提示词、设置、日志或 QA 产物；`patent-research-2026-09-02.md` 为用户本地文件，禁止加入发布提交。
- `docs/images/ai-checkmarks.jpg`、`in-place-translation.jpg`、`ai-drawing.jpg`、`code-explanation.jpg`、`web-annotations.jpg` 保留用户提供的原图，双语说明描述能力而非保证模型正确。
- 用户视频原编码为 HEVC；`docs/media/MewuAI-video-annotations.mp4` 转为 H.264/yuv420p 并保留原 AAC 音轨，以便普通浏览器和 Windows 播放。转换工具只用于本地文档处理，不进入产品依赖或安装包。
- Release 将演示 MP4 单独上传；应用 EXE/ZIP 仍不得携带文档图片和视频。更新器的 SHA256SUMS 只覆盖两个版本化安装/便携资产，不把演示文件当更新包。
- README 配图使用仓库内相对路径，演示视频链接绑定已发布的版本资产；README、安装包、csproj 与发布说明必须版本一致。
- 自包含发布前必须单独按 `Release`、`x64`、`win-x64`、`SelfContained=true` 执行锁定还原，再以同样参数 `publish --no-restore`。普通解决方案还原不会保证下载 .NET/WPF Runtime pack；本机已有缓存会掩盖此问题，需用全新包目录验证。v0.2.0 在 CI 因 NETSDK1112 失败，保留原标签不移动，正式包改由 v0.2.1 发布。
- NuGet 自定义包目录不保证 `NuGetPackageRoot` 带结尾分隔符，许可证路径必须显式加入目录分隔符；全新缓存的发布演练已覆盖该差异，不能只依赖默认用户缓存。
- `workflow_dispatch` 用于新标签前的完整云端预检：编译、测试、publish 审计和安装包打包全部照常，只跳过 GitHub Release 创建。v0.2.1 被真实录屏取消测试拦住，保留失败标签，后续发布用 v0.2.2。
- v0.2.3 包含 Hermes 启动兼容、四提供商简化配置、密钥掩码、请求体参数、统一连接提示及贴图重截图阴影修复；README 继续保留 GIF 与功能图库，发行日志仅列双语修复说明。版本更新需同步 ReleasePreparationTests 及对应发行说明测试夹具。打标签前先通过 workflow_dispatch 全流程预检，正式标签不得复用。
- GitHub Windows runner 默认英语；诊断文本测试必须按实际产品语言分别断言中英文类别，不能只匹配中文导致本机通过、CI 失败。隐私断言（不泄漏原始输出）与未知退出码不能等同安装损坏的断言仍必须保留，不能通过强制 CI 中文或删测试绕过。
