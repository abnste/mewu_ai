本次更新改善长回答阅读与操作响应，加入教学共享模式，并让设置页自动检查更新。

- 设置 → 捕获新增“教学演示模式”。开启后，下次截图的选区、批注和回答可被屏幕共享捕获，相关贴图与原位对话框同步可见；设置与凭据窗口继续防捕获。演示模式下停用自带区域录屏和长截图，避免将覆盖层录入结果。腾讯会议与极域的远端显示仍需在实际教学环境中验证。
- 合并流式增量，复用稳定的 Markdown 段落与截图裁剪，减少长回答逐字扫描、重复排版及鼠标移动时的无效布局；改善历史加载与已删除区域的媒体资源回收。
- 历史展开/收起恢复纯箭头；回到最新回复使用滚动条底部的小箭头，修正与滑块的对齐，不改变左侧正文布局。上翻阅读时保留当前位置，点击箭头后恢复跟随。
- 每次打开设置页自动检查官方 GitHub 更新。发现新版先询问是否下载，选择“稍后”不下载；下载及 SHA-256 校验完成后再确认安装并重启。关闭设置会取消检查和下载。

本地 Release 全量 760 项测试通过。性能改善基于本机合成回放；不代表所有设备或场景中的卡顿均已消除。公测模型仍为 MiniMax M3，其他模型请自行验证兼容性。

---

This update improves long-answer reading and responsiveness, adds a teaching screen-sharing mode, and checks for updates when Settings opens.

- Enable Teaching mode under Settings → Capture to make the next capture overlay, annotations, answers, related pins, and in-place dialogs visible to screen sharing. Settings and credential windows remain protected. Built-in region recording and scrolling captures are disabled in this mode to keep the overlay out of recorded results. Remote viewing through Tencent Meeting and Mythware still requires validation in the actual teaching environment.
- Batch streamed text, reuse stable Markdown blocks and image crops, and reduce per-character scans, repeated layout, and unnecessary work during pointer movement. Improve history loading and release media resources when deleted regions leave retained history.
- Restore arrow-only history controls. Place a small return-to-latest arrow below the answer scrollbar, aligned with its thumb without shifting the answer layout. Scrolling up preserves the reading position; clicking the arrow resumes following the latest response.
- Opening Settings automatically checks official GitHub releases. A new version prompts before downloading; Later leaves it undownloaded. Installation and restart require confirmation after the download passes SHA-256 verification. Closing Settings cancels checks and downloads.

All 760 local Release tests passed. Performance improvements were measured with synthetic local replay and do not imply that every latency spike is eliminated on all devices. The beta test model remains MiniMax M3; please verify compatibility with other models.
