# 喵呜AI 0.4.1 / MewuAI 0.4.1

## 多步计算与批改核对

- 修复手写多步计算被挤在同一行：识读、正确答案和批改说明支持回车、自动折行与有界滚动。新的批改请求要求保留原卷的逐行步骤；旧的连续等式仅调整空白排版，不改公式、负号或指数。
- 批改说明可直接编辑，确认题目时与判定一起保存；修正判定后不必继续保留矛盾的旧说明。教学 CSV 导出增加批改说明，并保留计算过程与说明中的换行。
- README 更换为官方公开的 HKDSE 真实手写答卷，展示逐题人工核对完成后的结果、分行计算和调整后的作答框，附原卷来源。原始扫描没有添加模拟答案；这张图不是无人审核的 AI 准确率证明。

验证：Release 构建无警告；1007 项测试通过，1 项现场音频测试按开关跳过；中英文真实覆盖层完成多行显示、修改识读/说明、调整框、确认和导出验证。遇到模型识读分歧仍保留待核，不能用排版修复代替老师复核。

## Calculation steps and reviewed grading

- Preserve multiline calculations in recognized answers, expected answers and explanations. Editors support Enter, wrapping and bounded scrolling. New grading requests preserve the original written steps; legacy equality-chain formatting changes whitespace only.
- Edit and save review explanations together with verdicts. Teaching CSV exports now include explanations and retain line breaks in calculations and notes.
- The README uses an official HKDSE handwritten script, showing the workflow after a reviewer checked both questions and corrected readings and answer boxes. The full original scan and its source are retained; the image does not represent unattended grading accuracy.

Validation: warning-free Release build, 1007 passing tests and one opt-in live-audio test skipped. Both UI languages passed the multiline editing, confirmation, answer-box adjustment and export replay. Model disagreements still require review.

## 下载 / Downloads

- `MewuAI-Setup-0.4.1-win-x64.exe`
- `MewuAI-Portable-0.4.1-win-x64.zip`
- 使用 GitHub 资产自带 SHA-256 digest，不附带 `SHA256SUMS.txt`。 / Uses GitHub asset SHA-256 digests; no separate checksum file.
