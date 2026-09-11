# 喵呜AI 0.4.3 / MewuAI 0.4.3

- MiniMax-M3 的普通对话、截图、表格及视频问答使用官方最大输出容量 524288 token，保留思考，移除原先固定的 8192 token 回复限制。其他兼容 API 不额外施加这个固定限制，沿用服务端默认；不宣称所有模型均可自动获取最大输出。
- 表格识别不再携带无关聊天记录和绘制说明，保持现有批注；思考过程正常显示，表格正文在完整结束后呈现，避免复制尚未接收完的表格。
- 原位翻译遇到明确的上下文超限、截断或超时会有界拆分补译。过长的单个 OCR 行也会分段并合回原位置，修正纯文字错误提示误提“缩短视频”的问题。
- 移除底部对话条中独立的“试卷与作业”按钮。试卷通过原有截图、多区域引用和普通 AI 问答进行分析；同步更新 README 操作说明，保留真实答卷的历史演示与人工复核说明。

验证：本地 Release 构建与 1045 项测试通过，1 项需现场音频设备的测试按开关跳过。请求参数与大表格传输使用 HTTP/SSE 回归验证，不代表所有渠道或真实试卷识别准确率。正式安装包由 GitHub Actions 重新执行全量测试、锁定依赖还原及发布包审计后生成。

---

- Ordinary conversations, image questions, table recognition and video questions use MiniMax-M3's documented maximum output allowance of 524288 tokens while retaining thinking, replacing the fixed 8192-token limit. Other compatible APIs retain their server defaults without that fixed application limit; automatic maximum discovery is not claimed for every model.
- Table requests omit unrelated conversation history and drawing instructions and preserve existing annotations. Thinking remains visible; table content appears after the response finishes so an unfinished table is not presented for copying.
- In-place translation retries explicit context-limit failures, truncation and timeouts using bounded smaller sections. Oversized OCR lines are split and reassembled at their original location. Text-only errors no longer advise shortening a video.
- Remove the separate Papers and assignments button from the conversation bar. Analyze papers through the existing screenshot, multi-region reference and AI conversation workflow. Update README instructions while retaining the genuine exam example as a clearly labeled, human-reviewed historical demonstration.

Validation: local Release build and 1045 tests passed; one opt-in live audio test skipped. HTTP/SSE regression tests verify request parameters and large-table transmission, not recognition accuracy across all providers or real exam papers. GitHub Actions reruns the full tests, locked restore and package audit before creating official packages.

Downloads: `MewuAI-Setup-0.4.3-win-x64.exe` and `MewuAI-Portable-0.4.3-win-x64.zip`. No `SHA256SUMS.txt`; use the SHA-256 digests provided by GitHub for each asset.
