# MewuAI 1.0.2

- 减少截图唤起、连续框选和会话切换中的重复保存与完整状态拷贝。截图背景提前解码，仍在原始尺寸的图片就绪后显示。
- 数据库日志整理移到独立后台连接，合并连续操作的日志整理，减少重复磁盘同步；保留每次提交的持久保存、失败回滚与历史导出的一致快照。
- 减少大量历史会话时的数据库页面重写；窗口改用轻量修订通知合并读取状态，避免重复传送整份历史。绘制保存后保留几何未变的 SVG 节点。
- 连续绘制按顺序保存，撤销、重做与退出等待已接受的笔迹；多个选区更新时保留已有对象和工具条。
- 改进鼠标释放事件缺失时的分笔处理，新一笔不再沿用上一笔的采样点；触笔、触控和对象拖动保持各自的输入规则。
- 连续框选时保留尚在保存的选框，避免开始下一框后上一框短暂消失；保存失败单独提示，操作工具仍等待真实保存完成。
- 修复鼠标经过旧选区时误清除待保存选框。长截图和录屏切换保留原画布，窗口加载完成后再交接；Windows 录屏保留倒计时与选区外暗罩，遮罩就绪后才开始采集。
- Windows 自动吸附保留悬停描边，原地点击松手后确认；直接拖框不再先提亮整个窗口。长截图保留选区边框、外圈暗色遮罩和随滚动增长的旁侧预览。
- 关闭 Windows 采集控件的系统出现动画，录屏控制条保持在截图遮罩上方；补齐采集期间的鼠标穿透样式，结束后恢复原窗口状态。
- Windows 全屏或接近全屏录制时仍显示录屏控制条，控制条单独排除出录制画面；修复录屏失焦时出现系统标题栏的问题。
- 支持 MiniMax-M3.1-Flash 正式版的截图标注参数兼容。
- 修复远程鼠标先报告按键释放时，画线、移动和框选被误取消的问题，覆盖视频批注；切换选区与媒体对象时统一工具条归属，鼠标经过选区不会中断视频播放或编辑。
- 视频播放与剪辑控件空闲时不再拦截 Esc，可直接退出截图；正在拖动剪辑或选择标注时仍先取消局部操作。
- 截图与黑板使用三个固定颜色槽，默认红、黄、蓝。单击切换，选中后再次单击换色；已有自定义颜色保持。
- 文字支持加粗、斜体、下划线和删除线，改善中文样式渲染。保存图片可选择「带标注图」或「原图」。
- 黑板打开、会话读取、取消回复和视频批注保存移出界面线程。迟到的取消请求不能取消同一会话中的新回复，视频批注按提交顺序发布。
- Windows 可在软件运行时接管 Win+Shift+S，默认开启，可在截图设置关闭；退出软件后恢复系统截图。
- 补齐 macOS 快捷键编辑，Wayland 使用系统 Portal 配置和修改截图快捷键。
- 改善 Windows 设置与录屏窗口创建失败后的清理，并在有原生错误码时显示诊断，便于定位不同设备上的兼容问题。

## 更新

通过「设置 → 关于」检查 GitHub 最新正式发行。安装包与版本签名校验保持，不需要额外更新清单或旧更新通道。

Windows 安装版、便携版，macOS ARM64 安装包和 Linux x64 AppImage 均使用同一版本源码。macOS 更新后可能需要重新确认屏幕录制权限；Linux 使用系统 Portal 授权。

旧 0.7.4、1.0.0 和预览版可能仍使用旧更新入口，请安装本发行一次，无需先卸载。支持的旧版 API 配置仍可导入；旧数据保留，旧历史对话尚未转换为新版历史界面。功能和平台范围仍见 [1.0.1 说明](./release-notes-1.0.1.md)。

## English

- Reduces repeated state writes and whole-state copies during capture, region selection and session switching. Background decoding starts earlier; presentation still waits for the original-sized image.
- Moves WAL checkpoint maintenance to a separate connection and batches maintenance across consecutive operations to reduce redundant disk synchronization, while retaining durable commits, rollback and consistent history exports.
- Reduces database page rewrites with large histories, coalesces full-state reads behind small revision notices, and preserves unchanged SVG path nodes after drawing saves.
- Saves continuous drawing in order and preserves existing objects and toolbars across selection updates.
- Separates ordinary mouse strokes when a release event is missing, without sharing the previous stroke's samples. Pen, touch and object gestures keep their existing input rules.
- Keeps pending selection outlines visible while the next selection starts, reports individual save failures, and enables tools only after the selection is actually saved.
- Prevents hovering an older region from clearing a pending selection. Scrolling capture and recording retain the original scene while their controls load. Windows recording shows its countdown and outside dimming before acquisition starts.
- Windows snap selection keeps its hover outline and confirms on a stationary release, avoiding full-window highlighting when dragging. Scrolling capture retains the region border, outside dimming and growing side preview.
- Disables Windows capture-control show transitions, keeps recording controls above the retained overlay, and restores the original window input styles after capture.
- Preserves strokes, moves and marquee selection when remote mouse input reports released buttons before pointer-up, including video annotations. Keeps region and media toolbars mutually exclusive without interrupting video playback or editing on hover.
- Idle playback and trim controls let Escape close the capture space; active trim gestures and annotation selection still cancel locally first.
- Keeps the Windows recording toolbar visible for full-screen and near-full-screen capture while excluding only the controls from captured frames, and prevents system title-bar painting after focus changes.
- Supports screenshot annotation parameter compatibility for the stable MiniMax-M3.1-Flash model.
- Screenshot and blackboard share three fixed color slots, initially red, yellow and blue. Click to select; click the selected slot again to edit. Existing custom palettes are retained.
- Adds bold, italic, underline and strikethrough text, improves styled Chinese rendering, and separates annotated and original image export.
- Moves board opening, session reads, reply cancellation and video annotation persistence off the UI thread. Late cancellation cannot cancel a newer run; video edits publish in commit order.
- Windows can handle Win+Shift+S while MewuAI runs. The option defaults on, can be disabled in capture settings, and releases the shortcut when the app exits.
- Supports shortcut editing on macOS and uses the system Portal to configure or change the capture shortcut on Wayland.
- Improves failed Settings/recording-window cleanup and reports native error codes when available for device-specific diagnosis.

Updates continue to use the latest stable GitHub Release directly, with package and version signature verification. macOS may request screen-recording confirmation again; Linux uses the system Portal. Earlier 0.7.4, 1.0.0 and preview installations may need one manual installation without uninstalling. Legacy data and platform limitations remain as described in the [1.0.1 notes](./release-notes-1.0.1.md).
