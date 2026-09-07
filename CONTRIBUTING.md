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

```powershell
dotnet restore .\mewu_ai_Assistant.slnx --locked-mode
dotnet build .\mewu_ai_Assistant.slnx -c Release -p:Platform=x64 --no-restore -warnaserror
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64 --no-build --no-restore
```

English contributors are welcome. Please include the same environment, reproduction, privacy, and validation details in English when possible.

