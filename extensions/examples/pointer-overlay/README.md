# 指针跟随十字标（pointer-overlay）

一个最小瞬态指针弹层示例。它注册一个 `space-toolbar` 命令；点击后通过
`ctx.ui.transient({ html, width, height })` 在指针位置弹出半透明十字标尺，
标尺随指针移动，实时显示当前坐标，抬起指针（或按 Esc、窗口失焦）即销毁。

- 权限：`ui.overlay`（一方法一权限，只覆盖 `ui.transient`）。
- 内容全部为自造 SVG，无任何外部素材或 Riot 资源。
- 弹层对指针完全透传（`pointer-events: none`），底层画布的选区拖选不被阻断；
  指针样本经 `mewu.on('pointer', ({x, y, buttons}) => ...)` 送达，坐标为空间窗口
  CSS 像素。

打包与发布流程与其他示例一致，参见
[插件打包工具](../../../tools/extension-market/README.md)。
