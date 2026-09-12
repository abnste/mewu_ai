# 参与贡献 / Contributing

感谢你帮助改进喵呜AI。提交 Issue 或 Pull Request 前，请先阅读[社区行为准则](./CODE_OF_CONDUCT.md)。

## 提交 Issue

- 使用 Bug 模板或功能建议模板，说明 Windows 版本、喵呜AI版本和复现步骤。
- 日志、截图和录屏必须先脱敏；不要提交 API Key、Cookie、凭据或真实用户屏幕内容。
- Hermes、WorkBuddy 或其他 AI 后端问题请同时注明后端版本和模型名称。

## 提交 Pull Request

1. 从 `master` 的最新提交开始修改。
2. 保持改动聚焦，并说明用户可见的行为变化。
3. 在 Windows x64 上运行相关 Release 构建和测试。
4. 更新受影响的 README、发布说明或许可证声明。
5. 在 PR 描述中写明验证结果和已知限制。

项目使用 MPL-2.0。提交代码即表示你同意该代码按 MPL-2.0 提供，并保留已有版权和许可证声明。请不要引入 GPL/AGPL 依赖或未授权的第三方内容。

## 本地验证

内部研发备忘、agent 指令、用户配置、凭据、原始调试记录及宣传制作目录仅保留本地，不进入提交或发行包。提交使用 GitHub 提供的隐私邮箱；不要把个人邮箱或电脑名写入 Git 作者信息。第三方许可证和必要作者署名必须保留。

提交前运行 `pwsh -NoProfile -File ./scripts/Test-Privacy.ps1`。检查会拒绝私有文件，并使用固定版本、校验过哈希的 Gitleaks 扫描提交历史；检测结果不输出凭据原值。首次运行需要从官方 GitHub 下载扫描器。相同检查会在 Pull Request、主分支更新和正式发布前运行。

```powershell
dotnet restore .\mewu_ai_Assistant.slnx --locked-mode
dotnet build .\mewu_ai_Assistant.slnx -c Release -p:Platform=x64 --no-restore -warnaserror
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64 --no-build --no-restore
```

English contributors are welcome. Please include the same environment, reproduction, privacy, and validation details in English when possible.
