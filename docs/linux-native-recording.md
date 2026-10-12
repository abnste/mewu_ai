# Linux 录屏验收契约

当前生产录屏仍未准入。原始像素入口的源码准备、跨平台类型检查或编码器清单不能作为实际录屏、音频、安装或 GUI 通过的证据。

X11 帧源复用已有 RandR 拓扑版本、ZPixmap 字节序/颜色掩码/行填充解码和真实窗口隐藏订阅。宿主必须从现有 `CaptureWitness` 绑定真实 display、两个 Mewu drawable 及其生命周期；前端不能提供或覆盖这些身份。父进程继续检查见证代际，子进程保留同一 X11 连接和事件队列，拒绝销毁、重建父子关系或短暂 map 后 unmap。仅最终 `map_state` 为隐藏不足以证明整个采集期间隐藏。

每帧只请求选区的确切 `GetImage` 矩形，最多 400 万像素，并在采集前后检查完整显示器拓扑版本和隐藏状态。坐标、显示器或窗口变化时停止，不能退回全屏。原始 RGBA 直接送固定编码图，不通过临时 PNG 编解码。Wayland 尚未接入 portal/PipeWire 授权，不能通过 XWayland 推断完整桌面可采集。

持续帧源可以在同一连接上重新设置单次请求的一秒上限，但始终截断到创建连接时的原始硬截止时间。普通截图仍保持原有十秒总预算。父 worker 的启动、进度、停止与总预算仍独立生效，不能因为取消、超时或 terminal receipt 提前释放进程许可；真实 Child、PID1 域及 IO 线程回收才是完成证明。

声音模式沿用 `mute/system/microphone/both`。Mute 不枚举或激活音频设备。System 必须选择真实默认输出 sink 的 `monitor_source`，不能把默认输入 source 当作系统声音或静默改用麦克风。拟接入的 typed PulseAudio 客户端使用 NOAUTOSPAWN，固定已存在的本地 Unix 服务端，锁定设备身份并观察变化；没有服务或设备时给出明确结果，不安装或启动系统服务。录音设备丢失、默认路由变化、时间异常与溢出要沿用现有停止原因，并核对真实成品音轨。

编码、暂停时间轴、AAC 延迟/填充、MP4 最终写入、生产 host/factory 路由与 F8 GUI 契约仍需要独立实现和原生验收。GStreamer 接入必须使用固定 typed 元素与有界队列，和当前 GLib/构建系统兼容；不开放任意 pipeline、命令行程序或未验证的编码器回退。

原始帧源的隔离 Xvfb 测试需要显式执行，不采集用户桌面、不连接音频、不创建成品：

```sh
xvfb-run -a cargo test -p mewu-desktop --locked --features linux-x11-tests x11_visibility_native_recording_exact_region_and_transient_remap -- --ignored --nocapture --test-threads=1
```

该测试仅核对真正 X server 的选区尺寸、已知白色像素及瞬时 remap 拒绝。它不代替真正 GTK producer 的身份见证、编码成品或 GUI 测试。录屏原生验收还须覆盖默认系统声音已知音调、Mute 无连接、麦克风明确授权、Both 混合、暂停/继续、停止/取消、路由变化、编码过载、文件最终化、设备与线程真实退出，以及最终 source/hash 对应的安装与更新。

依据：[X11 协议](https://www.x.org/docs/XProtocol/proto.pdf)、[GStreamer X11 越界行为](https://github.com/GStreamer/gstreamer/blob/1.20.3/subprojects/gst-plugins-good/sys/ximage/gstximagesrc.c)、[GStreamer Pulse source](https://github.com/GStreamer/gstreamer/blob/1.20.3/subprojects/gst-plugins-good/ext/pulse/pulsesrc.c)、[PulseAudio 显式 server 与 autospawn](https://github.com/pulseaudio/pulseaudio/blob/v16.1/src/pulse/context.c)、[GStreamer 时钟](https://gstreamer.freedesktop.org/documentation/application-development/advanced/clocks.html)。实现与测试为 Mewu 自有 MPL-2.0 代码，不复制第三方源码或模型。
