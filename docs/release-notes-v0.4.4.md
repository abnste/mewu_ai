# 喵呜AI 0.4.4 / MewuAI 0.4.4

- 加快原位翻译：多个文本批次可并行处理，结果仍对应各自原文；实际速度取决于模型、网络和页面内容。
- 改善译文定位：以 OCR 原文起点排版，减少换行、字体留白造成的偏移；复制、贴图和带标注保存保留译文位置及选区空白边距。
- 修复贴图置顶后再次截图时的窗口层级，确保新截图覆盖层能正常框选和操作。
- 调整主界面的防捕获设置，减少与第三方录屏工具的冲突；设置和凭据窗口仍保持保护。NVIDIA 即时重放的实际表现仍需对应硬件验证。
- 双语 README 将试卷批注与视频 GIF 纳入六宫格。试卷仍通过普通截图问答使用，保留真实答卷演示的人工复核及来源说明。
- 加强源码和发行包的隐私检查，阻止内部资料、用户配置和凭据进入发布内容。

译文定位仍依赖 OCR 的识读和字框精度。此版本未改变 AI 思考设置或回复预算。

---

- Speed up in-place translation by processing text batches concurrently while keeping each result tied to its source. Actual latency depends on the model, network and page content.
- Improve translated text placement using OCR start positions and compensate for wrapping and font margins. Copying, pinning and annotated exports preserve translation positions and empty selection margins.
- Fix window stacking when taking another screenshot over pinned images, keeping the new capture overlay interactive.
- Adjust capture protection on the main window to reduce conflicts with external recording tools. Settings and credential windows remain protected. NVIDIA Instant Replay still requires validation on corresponding hardware.
- Combine paper annotations and the video GIF with the existing previews in a six-cell bilingual README gallery. Papers continue to use ordinary screenshot conversations, with the reviewed example and source attribution retained.
- Strengthen repository and release privacy checks to prevent internal records, user settings and credentials from entering published content.

Translation placement still depends on OCR recognition and bounding-box accuracy. AI thinking settings and response budgets are unchanged.

Downloads: `MewuAI-Setup-0.4.4-win-x64.exe` and `MewuAI-Portable-0.4.4-win-x64.zip`. SHA-256 digests are available in the GitHub asset details; no separate checksum file is included.
