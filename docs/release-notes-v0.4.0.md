# 喵呜AI 0.4.0 / MewuAI 0.4.0

## 试卷与作业

- 对话条新增“试卷与作业”：导入 PDF 指定页、图片，或收集当前多选区；关闭覆盖层翻页后，可再次截图继续收集。页面仅在本次应用会话保留，点击批改才发送。
- 作答代号与页码独立绑定；同一作答的重复图片、重复页码会被拦截。一次作业最多 16 页、3200 万像素，每批勾选 1–4 页逐页处理。
- 每页先批改，再核对原图、计算与初稿的一致性。分歧题标为待核；老师可修正识读、答案、知识点、判定、分数和框选位置。没有评分细则不生成成绩，仅汇总已确认且有评分依据的小问。
- 原卷使用简短勾叉、待核标记与作答框。本地 OCR 优先寻找唯一匹配的实际作答，保留正负号，避免吸附到题干。
- 按老师已核对的错题汇总至少两份不同作答的共同知识点；生成 4 道巩固题并独立验算，题目、答案和解析可编辑。确认后导出 ZIP，内含标注页、逐题 CSV、分开的可打印练习与答案 HTML。
- Windows PDF 渲染放在独立、无窗口的本地进程，避免与 WPF 图形设备共用进程时触发本机 AMD 驱动退出异常；主界面保持硬件加速。PDF 进程不读取设置或凭据，取消与超时会结束进程，不生成临时媒体文件。
- 修复普通回答题号在显示/复制时从 1 重排，以及未生成图片批注仍提示完成的问题。

这是教师辅助批改流程，仍需核对。公开 TSA 两份模拟作答复测中，最后一轮 12 项中 8 项判断符合参考，4 项识读/判断分歧进入待核；真实手写样本的两题也进入待核。不能据此推断整卷、全班或其他学科的准确率。不同渠道的视觉能力、额度与输出完整性仍会影响结果，失败页面不会伪装成批改成功。

使用说明与评估边界：[教学流程](https://github.com/abnste/mewu_ai/blob/v0.4.0/docs/teaching-workflow.md)。原卷版权归原机构，安装包不附带试卷或验收数据。

## Papers and assignments

- Import selected PDF pages or images, or collect screenshot regions across capture sessions. Assign submission IDs and page numbers; reject duplicate pages within a submission. Pages remain in memory and are sent only when grading starts.
- Collect up to 16 pages / 32 million pixels, grading 1–4 pages per batch. Each page receives an initial pass and a separate review against the original image. Disagreements remain uncertain. Teachers can correct readings, answers, skills, verdicts, rubric-based scores and answer boxes.
- Review shared errors from at least two distinct submissions, then generate four independently checked exercises. Edit and confirm the questions and answers before exporting annotated pages, a question-level CSV, and separate printable question/answer HTML files in a ZIP.
- Isolate Windows PDF rendering in a bounded local worker to avoid the reproduced AMD driver shutdown failure. Keep WPF hardware acceleration enabled. The worker does not load settings or credentials and creates no temporary media files.
- Preserve original question numbering in displayed/copied answers and report missing image annotations accurately.

Teacher review remains necessary. In the small public TSA sample, the final replay had eight of 12 item states matching the reference and four flagged uncertain; both real handwriting items were uncertain. These results do not establish unattended grading accuracy. Vision-service availability and output limits still apply.

## 下载 / Downloads

- `MewuAI-Setup-0.4.0-win-x64.exe`：当前用户安装 / Per-user installer.
- `MewuAI-Portable-0.4.0-win-x64.zip`：免安装包 / Portable package.
- 使用 GitHub 资产自带 SHA-256 digest，不附带 `SHA256SUMS.txt`。/ Uses GitHub asset SHA-256 digests; no separate checksum file.
- Windows 10 2004+ / Windows 11，x64；包内包含 .NET 运行时、离线 OCR 及许可文件。/ Includes the runtime, offline OCR and license notices.
