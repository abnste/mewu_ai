# Mewu Desktop Witness

GNOME 50 的区域操作见证组件。**输入和屏幕共享仍由 XDG Desktop Portal 授权。**此组件仅提供同一次合成器观察中的窗口生命周期标识、几何、堆叠、焦点、坐标命中及显示器布局。窗口标题只在本机计算摘要供宿主匹配辅助功能接口，不返回标题原文、应用文本或截图，也不提供输入注入、脚本执行或窗口操作接口。

固定的 Escape 停止快捷键仅在原生 Mewu 进程持有租约时注册。调用者 D-Bus 连接退出、10 秒失联、10 分钟到期、组件停用或显式结束均释放快捷键；停止后同一租约不能重新开始。原生宿主还必须验证 GNOME 的 D-Bus 唯一所有者、进程身份、Portal 会话、窗口及每次输入的焦点边界，不能把此组件当作输入授权。

版本 2 增加同一有效租约下的合成器重绘请求。静止桌面可能不再产生新 PipeWire 帧，宿主在建立时间屏障后请求一次重绘，随后仍须从原 Portal 流取得符合时间屏障的新帧。该请求不返回像素、不改变窗口和输入焦点，锁屏、系统模态界面、概览及已停止的租约均拒绝；不能用旧截图冒充操作反馈。

该适配仍在开发验收中。开发构建包含 Linux x86_64 宿主适配，但社区区域插件仍未声明 Linux 支持，须完成真实桌面验收后才能发布。其他 GNOME 版本及其他 Wayland 合成器不在当前声明范围。键盘输入要求应用提供可验证的辅助功能角色和窗口几何；GTK4 不能可靠区分密码框的版本会拒绝自动输入，不会根据屏幕文字猜测保护状态。

契约检查：`node --experimental-vm-modules --test tools/linux-desktop-witness/*.test.mjs`。覆盖实际组件方法与租约策略，不代替 GNOME 桌面、Portal 授权及真实输入的端到端验收。

实现依据：[GNOME D-Bus](https://gjs.guide/guides/gio/dbus.html)、[Meta.Window](https://mutter.gnome.org/meta/class.Window.html)、[XDG RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)。
