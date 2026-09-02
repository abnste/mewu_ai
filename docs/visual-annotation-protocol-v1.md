# 喵呜AI 视觉标注协议 v1

协议标识：`mewu.visual-annotations/1`

本协议用于模型对图片和视频返回可执行的视觉标注。结构参考 W3C Web Annotation 的“标注体—目标—选择器”模型：`target` 绑定本轮附件，`geometry` 选择空间区域，`timeline` 选择视频时间并提供随时间变化的几何，`kind/content/style` 描述要执行的标注。

```json
{
  "annotationProtocol": "mewu.visual-annotations/1",
  "answer": "批改说明",
  "annotations": [{
    "id": "mark-1",
    "target": { "regionIndex": 0, "referenceHandle": "ref-..." },
    "kind": "pen",
    "geometry": { "coordinateSpace": "normalized", "points": [{ "x": 0.12, "y": 0.31 }, { "x": 0.18, "y": 0.38 }] },
    "style": { "color": "#E53935", "strokeWidth": 0.006, "opacity": 1 },
    "label": "这一步符号写错"
  }]
}
```

## 目标

- `regionIndex`：从 0 开始的完整附件上传顺序，图片和视频混排。
- `referenceHandle`：发送清单中的不可变句柄，是主键。句柄与序号冲突时以句柄为准；未知或重复句柄拒绝。
- 图片不得带时间轴，视频必须带时间轴。

## 图元

`kind` 仅允许 `callout`、`pen`、`highlighter`、`rectangle`、`ellipse`、`arrow`、`text`、`number`、`mosaic`。它们分别对应可拖动说明气泡、自由画笔、高亮笔、矩形、椭圆、箭头、文字、实心序号和矩形像素化。

矩形类几何使用 `rect:{x,y,width,height}`；路径类使用 `points:[{x,y},...]`。所有值基于附件自身尺寸归一化到 0–1。矩形必须具有正宽高并完全位于附件内；路径为 2–128 个有限点。文字正文位于 `content.text`；序号位于 `content.number`，范围 1–999。

## 视频时间轴

视频标注不使用顶层 `geometry`，而使用 `timeline:{startTime,endTime,keyframes:[{time,geometry}]}`。时间单位为秒。单点事件令起止时间相等并提供一个关键帧；区间至少两个严格递增关键帧。矩形与点数相同的路径在相邻关键帧间线性插值。

## 样式与安全界限

- `color`：`#RRGGBB`；`strokeWidth`：相对短边的 0.001–0.1。
- `opacity`：0.05–1；`filled`：形状是否填充；`fontSize`：相对高度的 0.01–0.2。
- 单次最多 48 条标注；单条路径最多 128 点。
- 严格逐条校验；非法兄弟项不影响正文和其他合法标注。
- 旧版扁平矩形/视频批注仍可读取，但新请求只要求 v1。
