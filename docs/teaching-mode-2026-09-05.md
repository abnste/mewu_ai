# 教学共享修复（2026-09-05）

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
