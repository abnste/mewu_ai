# 录屏实现备忘

- 录屏使用 ScreenRecorderLib + Windows Media Foundation，输出文件由 `RecordingSession.VideoPath` 统一管理。
- 覆盖层录制时，选区内部从冻结桌面/遮罩中挖空；窗口原生区域同步排除选区内部，使其他进程的窗口可以继续播放并接收鼠标输入。
- 录制完成回调返回的最终路径必须先同步到 `VideoPath` 并转移 `TempMediaRegistry` 租约；完成事件要等待文件大小稳定，覆盖层保持“处理中”直到录制会话释放 Media Foundation 句柄，再退出录制模式并启动同一覆盖层内的 WinRT `VideoPreviewSurface` 原位自动播放，避免先露出空白预览。
- 录制期间 `SetWindowRgn` 的挖空区域必须按实时 HWND 外框计算相对坐标，统一使用 `ScreenCoordinateService.ToWindowRelativePixelRect` 处理虚拟桌面负坐标、窗口原点偏移和边缘裁剪；从 `WM_NCHITTEST.lParam` 读取真实屏幕点，布局短暂无效时保留上一次穿透区域，避免底层视频被遮回。
- Debug 视觉验收模式仍禁止录屏，以避免绕过防捕获保护；Release 不提供运行时绕过开关。
- 原位视频预览使用 WinRT 帧服务器时，预览表面的长边限制为 1280 像素、最高接收 15 FPS；WPF 渲染队列已有待显示帧时必须在 GPU 读回前丢帧，并复用 `WriteableBitmap`，避免 1080p/4K 视频造成持续大对象分配。该限制只影响预览，不修改源 MP4。
- 首次无法建立 `SetWindowRgn` 录制穿透孔洞时必须拒绝开始录屏；结束录屏恢复完整窗口区域失败时只在成功后清状态，并做至多三次渲染优先级重试，仍失败则关闭覆盖层销毁 HWND，不能让旧孔洞永久残留。
- 原位视频标注 seek 必须使用 `MediaPlaybackSession.Position` 并等待 `SeekCompleted` 后再显示目标框；区间播放的跟踪刷新绑定实际呈现帧的位置，终点有界停止。播放器替换或释放时必须解绑帧回调，generation、取消令牌和当前选区共同拒绝旧播放器的迟到更新。
- 录屏倒计时只负责原位视觉与底层交互，不能提前创建或启动 `RecordingSession`。倒计时期间用可恢复的窗口级鼠标穿透保持跨进程操作，固定三步且支持取消；数字隐藏并恢复窗口输入后才进入现有录屏模式，确保成片首帧不含倒计时。
