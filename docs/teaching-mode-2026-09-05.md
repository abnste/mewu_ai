# 教学共享修复（2026-09-05）

> 下文保留 2026-09-05 首次实现及当时验证记录。2026-09-08 已解除教学模式下录屏/长截图的限制，当前实现见本文末尾。

基于 `a47040b` 继续修改，已核对 origin/master 当前无新增提交；本轮未发布或修改版本号。

## 问题与修改

Windows `WDA_EXCLUDEFROMCAPTURE` 会让会议、课堂广播等使用系统捕获能力的软件看不到喵呜AI覆盖层。原先所有窗口一律防捕获，没有适合教学的选择。

新增默认关闭的 `TeachingMode`。在“设置 → 捕获 → 教学演示模式”开启并保存后，下一次截图的框选、标注、对话、原位颜色/保存对话框，以及该覆盖层新建的贴图/贴视频允许被系统捕获。请共享整个屏幕；只共享底层某个应用窗口不包含其他顶层窗口。

模式在覆盖层创建时固定，现存覆盖层和贴图不会突然改变共享状态。设置/凭据页继续防捕获；Release 仍然忽略 Debug QA 环境变量。教学模式下自带区域录屏和长截图不可用，入口和执行函数均拦截，界面明确说明切换方法；课程录像可使用会议软件的录制功能。

覆盖层激活和贴图后刷新背景时，短暂开启并核验防捕获，等待 DWM 完成当前提交再获取干净桌面，并在 finally 恢复共享。无法恢复时关闭覆盖层；捕获失败保留上一帧。教学贴图已经被系统截到，不重复合成半透明内容和阴影。

## 验证

- Release 解决方案构建成功：主程序、测试、ProviderSmoke，0 警告、0 错误。
- Release 默认测试共 748 项通过；新增覆盖旧配置默认值和教学偏好开启/关闭的保存、重新加载。
- `tests/InteractionHarness --verify-teaching` 在 Release 使用真实 HWND + GDI 捕获，以纯绿底窗和品红覆盖层比较像素。确认共享可见、干净背景刷新、刷新后恢复可见、录屏拦截、设置页受保护、教学贴图可见且不二次合成、普通贴图仍受保护。
- Release `--teaching` 通过 Windows.Graphics.Capture 视觉检查，选区、浅色工具条和教学标识可见。测试仅用合成内容，不读取真实设置/凭据，不发起 AI 请求。报告位于忽略目录 `.codex-build/teaching-verification.json`，不提交截图。
- 尚未连接腾讯会议或极域的另一台接收端，不将本地捕获验证宣称为实际远端会议验证。

## 官方依据

- [SetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)：`WDA_NONE` 允许捕获，`WDA_EXCLUDEFROMCAPTURE` 排除窗口。
- [DwmFlush](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmflush)：等待当前应用已排队的 DirectX 更新提交。

## 2026-09-08：教学共享与本机采集共存

用户明确要求教学模式保持开启，同时使用内置录屏和长截图，取代首次实现的一律禁用策略。默认防捕获和设置/凭据窗口的保护保持原有行为。

教学采集期间保持 `WDA_NONE`；通过 `SetWindowRgn` 从覆盖层实际 HWND 区域排除完整采集矩形，让底层实时画面与鼠标交互通过。采集开始前以 `GetWindowRgn` / `RectInRegion` 验证没有覆盖层像素落入采集区；长截图每次取帧及录屏计时回调继续验证，失败时停止。普通模式沿用窗口防捕获，不能用此路径绕过 Debug QA 的录屏限制。

选区外保留指示框，控制条和长截图预览有限次寻找区外空位，并避免互相覆盖。全屏等无空位时隐藏控件，使用会话期间注册的全局 F8 停止录屏或完成长截图；倒计时期间 F8 取消，长截图仍支持 Esc 取消。热键占用时不启动采集，所有取消、失败与关闭路径恢复窗口区域并释放热键。

验证使用自有彩色滚动页面，不加载用户配置或调用 AI。Release 全量测试 902 项通过、1 项真实音频设备测试跳过；普通模式走通长截图启动/取消、录屏倒计时/停止和 MP4 首中末帧像素检查。教学模式额外验证共享保持可见、无孔洞时拒绝采集、滚动追加、取消恢复、F8 取消倒计时，以及全屏长截图完成和全屏录屏停止。共享回归继续验证背景刷新、设置页保护、教学贴图可见和普通贴图防捕获。这些是本机证据，不代表腾讯会议或极域的远端接收端已实测。

官方依据：[SetWindowRgn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowrgn) 明确窗口区域外不会绘制；[GetWindowRgn](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrgn) 与 [RectInRegion](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-rectinregion) 用于验证实际裁剪范围。所有坐标仍统一经 ScreenCoordinateService 处理。
