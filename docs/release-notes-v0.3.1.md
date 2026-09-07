这次更新改善了视频理解失败时的提示。此前服务端返回 HTTP 422 时，界面只能显示一个状态码，无法判断是文件、参数、上下文还是服务端临时问题。

### 视频请求提示

- 失败信息现在会明确显示本次实际使用的 AI 渠道，避免设置中的默认模型和对话条当前模型混淆。
- 对服务端能识别的错误，会给出对应的处理方向：视频或请求体过大、视频无法解码、附件编码异常、上下文过长、认证或额度问题。
- MiniMax 仅返回“视频内容”拒绝而没有详细代码时，会提示可能是内容审核或当前服务能力限制，避免把问题误导成文件大小。
- MiniMax 返回服务代码或追踪编号时会一并显示，便于继续排查或向服务方反馈。
- 不会把服务端原始错误正文直接显示或写入诊断，以免其中的提示词、视频数据或认证信息被泄露。

### 兼容性

- MiniMax M3 的 OpenAI-compatible 视频请求继续使用官方支持的 `video_url`、Base64、MP4、`fps: 2`、流式回答和思考参数。
- 已用 27 秒、1128×632、H.264/AAC 的合成 MP4 走完整流式请求验证；这项检查不会上传用户的录屏。

[全部提交变化](https://github.com/abnste/mewu_ai/compare/v0.3.0...v0.3.1) · [完整版本记录](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)

---

This update makes video-request failures understandable. Previously, an HTTP 422 response only showed its status, leaving it unclear whether the issue was the file, request parameters, context, or a temporary provider-side failure.

### Video request feedback

- Failure messages now identify the AI channel that actually handled the turn, avoiding confusion between the configured default and the current conversation selection.
- Recognized provider errors give practical guidance for an oversized video/request, video decoding, attachment encoding, context length, authentication, or quota.
- When MiniMax only reports a video-content rejection without a detailed code, the app explains that it may be a content-policy or service-capability decision instead of suggesting needless compression.
- MiniMax service codes and safe trace IDs are shown when available, making follow-up investigation or provider support possible.
- Raw provider error bodies are never shown or written to diagnostics because they can contain prompts, media data, or authentication information.

### Compatibility

- MiniMax M3 video requests continue to use its supported OpenAI-compatible `video_url`, Base64, MP4, `fps: 2`, streaming, and thinking parameters.
- A synthetic 27-second, 1128×632 H.264/AAC MP4 completed the full streaming request. This check did not upload a user's recording.

[Full comparison](https://github.com/abnste/mewu_ai/compare/v0.3.0...v0.3.1) · [Release history](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)
