# 安全策略 / Security Policy

## 支持版本

安全修复优先提供给最新 GitHub Release。旧版本可能已经停止维护，请先升级到最新版本后再确认问题是否仍然存在。

## 报告漏洞

请使用 GitHub 仓库的私密安全报告入口提交漏洞，不要在公开 Issue、Pull Request、截图或日志中披露细节。
报告应包含受影响版本、Windows 版本、最小复现步骤、影响范围和脱敏证据。请删除 API Key、Cookie、token、凭据、个人数据和屏幕内容。

维护者会确认收到报告，评估影响并在修复可用后更新公开说明。请不要在漏洞修复前公开利用代码或可直接复现的敏感样本。

## 设计上的安全边界

- API Key 和敏感认证 Header 使用 Windows DPAPI 本地保护。
- 纯文字对话不会自动附加桌面截图；只有用户明确选择视觉内容时才发送。
- 请把第三方 Provider 的数据处理和隐私政策纳入部署评估。

English reports are welcome. Use GitHub's private security reporting flow and redact secrets, tokens, credentials, personal data, and screen content.

