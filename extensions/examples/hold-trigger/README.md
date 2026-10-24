# 按住型触发器示例

注册一个 `Alt` 按住型触发器。在空间主窗口按住 `Alt` 即收到 `down`，松开收到 `up`（携带指针坐标与剩余修饰键），触发期间按 `Esc` 或窗口失去焦点收到 `cancel`。每个事件都用 `ctx.ui.render` 在画布盖一枚小徽章，肉眼即可确认事件到达；徽章是普通空间对象，可移动或删除。

触发器只承载修饰键：注册期拒绝字母、数字等一切非修饰键（宿主运行时校验），事件里只有 `phase`、`reason`、指针状态与修饰键标志，绝不包含键入的字符，因此不能被用作键盘记录器。事件分发走普通插件调用管线，清单必须声明 `input.trigger` 权限；监听与打字冲突防护（忽略自动重复、输入法组合态）由宿主在空间窗口负责，插件无感知。

从 Mewu 仓库根目录打包：

```powershell
node tools/extension-market/cli.mjs pack extensions/examples/hold-trigger --out D:/packages/hold-trigger-0.1.0.zip
```
