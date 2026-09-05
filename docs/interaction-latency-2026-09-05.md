# 纯箭头恢复与长回答卡顿修复

基于 `d0314d2`，按用户明确反馈恢复原历史上拉/下拉纯箭头。“回到最新回复”仅保留向下箭头，文字只留在悬浮提示和无障碍名称中。本轮不再增加按钮文案，不改教学演示模式，也不发布新版本。

## 已定位的开销

- 每次回答变化替换整个 FlowDocument，导致稳定段落重复创建、选区与全文布局失效。
- Emoji.Wpf 的编辑器变化处理逐字符寻找 emoji，并在每次增量后重建整个 Text 缓存。只读回答不需要这条编辑器输入管线。
- 对话条每次测量先设 540 DIP，布局又设为 574 DIP，同时缩小再恢复 MaxHeight，迫使全文反复换行。
- 回答的 RichTextBox 禁用内部滚动，外面另套 ScrollViewer，导致长文档以全部高度参与排版；未选区的每次鼠标移动也重排对话条。

## 修复与边界

保留稳定 Markdown 顶层块和同一 FlowDocument，更新变化后缀。引用、脚注等含方括号的旧/新正文保守全部重建，防止后文改变前文语义。动作替换清除旧按钮与回调。只替换新增块中实际含 emoji 的 Run，保留彩色符号及原有选区复制处理；PlainText 按需从文档和 emoji 原始文字生成。

回答由 RichTextBox 的内部有限视口滚动，外层仅限高。统一测量和最终宽度；相同显示器上的鼠标移动不重排，显隐不变不重启动画。保留阅读位置、回到末尾、跨屏重定位、自动收纳与最初干净截图。

## 可复现性能证据

Release `tests/InteractionHarness --teaching --benchmark`，同一台机器、160 次更新、9010 字合成正文。只比较未并行运行测试/构建的前后样本；中间调试样本有资源争用，不作最终证据。

| 指标 | 修改前 | 修改后 |
| --- | ---: | ---: |
| 文字更新 P95 | 123.30 ms | 3.43 ms |
| 输入优先级心跳间隔 P95 | 788.00 ms | 50.44 ms |
| 完整回放耗时 | 87.07 s | 17.77 s |
| 最长心跳间隔 | 1045.89 ms | 775.32 ms |

最后一次诊断将最长峰值定位在第 116 次更新，约 775 ms；此前同构建回放也出现过约 1 秒峰值。P95 改善不能抹去这些峰值，具体原因仍需进一步分析，不能归咎于启动预热或宣称所有场景完全不卡。现场确认了纯箭头外观、原位滚动及向下箭头出现。本地报告在忽略目录 `.codex-build`，不提交桌面截图。

## 验证记录

Release 解决方案构建 0 警告、0 错误。新增 Markdown 增量等价、稳定段落保留、列表/标题/代码块/表格/引用重解析和动作替换测试，原有彩色 emoji 与复制文本测试通过。检查过程中一次全量回归 756/757 通过，录屏立即释放用例报告文件占用（RecordingCleanup: IOException），该组两项独立诊断随后均通过，表明它有偶发性，但尚不能视为修复；保留该偶发记录，本轮未修改录屏释放实现。

## 一手依据

- [微软 WPF 布局性能](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/advanced/optimizing-performance-layout-and-design)
- [WPF TextBoxBase 的 TextChanged 实现](https://github.com/dotnet/wpf/blob/main/src/Microsoft.DotNet.Wpf/src/PresentationFramework/System/Windows/Controls/Primitives/TextBoxBase.cs)
- [Emoji.Wpf RichTextBox 源码](https://github.com/samhocevar/emoji.wpf/blob/master/Emoji.Wpf/RichTextBox.cs)

针对本轮改动的 25 项 Markdown/交互回归全部通过，新增覆盖流式增长时保持选中文字、跨段换行及彩色 emoji 复制。测试中发现 Emoji.Wpf 静态字形字典不支持多个 STA 并行初始化；将两个字形测试类归入互斥集合，匹配产品单 UI 线程的实际使用方式，未修改/放宽测试断言。

最终验收：关闭桌面回放、修正字形测试的共享缓存隔离后，Release 全量 **758/758** 通过，解决方案再次构建零警告/零错误。已重新 fetch origin，当前分支相对 origin/master 领先 2 个已有提交、无落后提交；本轮在这两个本地修改之上继续，不覆盖其他更新。
