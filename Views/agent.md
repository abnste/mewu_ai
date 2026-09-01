# 视图实现备忘

- `AllowsTransparency=True` 的自绘圆角窗口不使用系统 DWM 阴影。主界面的柔和窗口阴影必须由圆角、无命中的独立同级 `Border` 绘制，内容 `Shell` 单独做圆角裁剪；严禁把 `DropShadowEffect` 重新挂到带 `Clip` 的 `Shell` 上，否则阴影会被裁掉，整个内容子树的 ClearType 也会失效。
- 透明窗口外框要为模糊阴影预留足够的透明边距，并同步增加窗口 `Width`/`Height` 与最小尺寸，保持实际内容壳尺寸不变；否则阴影会再次被 HWND 边界截断。
