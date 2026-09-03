# 喵呜AI 视觉标注协议 v1

协议标识：`mewu.visual-annotations/1`

本协议用于模型对图片和视频返回可执行的视觉标注。结构参考 W3C Web Annotation 的“标注体—目标—选择器”模型：`target` 绑定本轮附件，`geometry` 选择空间区域，`timeline` 选择视频时间并提供随时间变化的几何，`kind/content/style` 描述要执行的标注。

```json
{
  "annotationProtocol": "mewu.visual-annotations/1",
  "answer": "批改说明",
  "annotationMode": "append",
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

## 多轮更新

`annotationMode` 决定本轮结果如何更新已显示的 AI 标注：

- `preserve`：保留已有标注，本轮通常返回空 `annotations`，只继续回答。
- `append`：保留已有标注，并把本轮 `annotations` 作为新增标注合并、去重。
- `replace`：模型明确决定重新标注时，用本轮完整标注替换旧标注。

后续提问会把当前已显示标注扁平化进新的图片附件，并在附件清单以 `hasExistingAiAnnotations` 告知模型。空或全部无效的替换结果不会清空已有标注；未被本轮合法标注命中的其他附件也保持原状。旧响应未提供 `annotationMode` 时按 `replace` 兼容，但空标注仍不会擦除当前结果。

## 目标

- `regionIndex`：从 0 开始的完整附件上传顺序，图片和视频混排。
- `referenceHandle`：发送清单中的不可变句柄，是主键。句柄与序号冲突时以句柄为准；未知或重复句柄拒绝。
- 图片不得带时间轴，视频必须带时间轴。

## 图元

`kind` 仅允许 `callout`、`pen`、`highlighter`、`rectangle`、`ellipse`、`arrow`、`text`、`number`、`mosaic`。它们分别对应可拖动说明气泡、自由画笔、高亮笔、矩形、椭圆、箭头、文字、实心序号和矩形像素化。

矩形类几何使用 `rect:{x,y,width,height}`；路径类使用 `points:[{x,y},...]`。所有值基于附件自身尺寸归一化到 0–1。矩形必须具有正宽高并完全位于附件内；路径为 2–128 个有限点。文字正文位于 `content.text`；序号位于 `content.number`，范围 1–999。

## 视频时间轴

视频标注不使用顶层 `geometry`，而使用 `timeline:{startTime,endTime,keyframes:[{time,geometry}]}`。时间单位为秒。单点事件令起止时间相等并提供一个关键帧；区间至少两个严格递增关键帧。矩形与点数相同的路径在相邻关键帧间线性插值。单条时间轴最多 128 个关键帧，只应在目标边界或运动方向实际变化时增加关键帧；客户端会移除可由相邻端点线性还原的冗余帧。

## 样式与安全界限

- `color`：`#RRGGBB`；`strokeWidth`：相对短边的 0.001–0.1。
- `opacity`：0.05–1；`filled`：形状是否填充；`fontSize`：相对高度的 0.01–0.2。
- 单次最多 48 条标注、最多 12 条 `callout`；单条路径最多 128 点；单条文字/标签最多 500 个 UTF-16 字符。
- 严格逐条校验；非法兄弟项不影响正文和其他合法标注。
- 旧版扁平矩形/视频批注仍可读取，但新请求只要求 v1。

## 客户端质量后处理

- 图片中的文字型目标优先用离线 OCR 行框校准；原生可操作控件优先使用 UI Automation 真实边界。
- 对图片框使用 IoU 去重；`callout` 与同目标的 `rectangle/ellipse` 重叠时保留信息更完整的 `callout`。
- 对视频轨迹同时检查时间重叠率与采样帧 IoU，抑制同一事件的重复轨迹；轻微片尾越界可钳制，明显越界逐条拒绝。
- 说明气泡使用全局有界布局，同时避让所有目标框和已放置气泡；屏幕预览、图片导出、MP4/GIF 导出使用同一布局规则。
