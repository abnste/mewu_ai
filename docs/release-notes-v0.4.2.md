# 喵呜AI 0.4.2 / MewuAI 0.4.2

- 试卷批改的错因和正确答案直接显示在原卷作答旁，并随带标注图片导出；批注避开其他作答和密集文字，拥挤时保留简短标记与完整侧栏说明。
- 新增整页原卷查看与返回核对入口。识读、正确答案和批改说明支持公式预览与原文编辑。
- 本地数学排版支持分数、上下标、根式、多行对齐以及常用 LaTeX 分隔符；普通 AI 回复也能显示公式，复制保留原始表达式。不支持的公式保持可读原文。
- 英文批改明确要求生成英文知识点和说明；原始学生文字保持原语言。README 英文演示知识点已修正为 “Laws of indices”，配图保留官方真实手写答卷并注明人工复核。
- 增加 WpfMath / XamlMath.Shared 2.1.0 及其 MIT、Knuth/OFL 字体许可，随安装包分发并可从关于页查看。

验证：Release 全量 1025 项通过、1 项现场音频测试按开关跳过；真实答卷的中英文原位批注、公式编辑、确认、整页查看/返回及导出回放通过。此回放包含有依据的人工核对，不代表模型无需复核即可自动准确判分。

---

- Show feedback and correct answers beside the original handwriting, including annotated-image exports. Avoid other answers and dense ink; crowded pages retain compact markers and complete side-panel explanations.
- Add full-page viewing and return-to-review controls, plus formula previews and editable source for readings, expected answers and explanations.
- Typeset fractions, scripts, roots and aligned steps locally, including common LaTeX delimiters in AI replies. Copy preserves the source expression; unsupported formulas remain readable as text.
- Request English knowledge points and explanations for English grading while preserving the student's original language. Refresh the genuine handwritten exam examples with explicitly reviewed results and English “Laws of indices”.
- Package WpfMath / XamlMath.Shared 2.1.0 and their MIT and Knuth/OFL font licenses, available through About.

Validation: 1025 Release tests passed, one opt-in live audio test skipped; Chinese and English real-script UI replays cover annotations, formula editing, confirmation, full-page view/return and export. The replay includes documented human review and is not an unattended model-accuracy claim.

Downloads: `MewuAI-Setup-0.4.2-win-x64.exe` and `MewuAI-Portable-0.4.2-win-x64.zip`. No `SHA256SUMS.txt`; GitHub supplies asset SHA-256 digests.
