# AI 翻译与响应完整性

- 原位翻译必须按 OCR 行身份映射，不能把合并段落的译文按数组下标强行贴回屏幕。InPlaceTranslationService 使用有界批次、固定行编号、二分补译以及单行纯文本回退；每次请求都必须声明目标语言并关闭思考。取消后不得继续补译或返回迟到结果。
- TranslationResponseParser 使用 System.Text.Json 校验完整结构；不能从尚未闭合的根对象中抽取内层数组作为完成信号，也不能把无关根对象的示例数组当作译文。编号映射须完整、唯一且位于当前批次范围内。
- OpenAI-compatible 的 finish_reason=length 表示输出截断，不能因为随后出现 [DONE] 而返回成功；先消费末块正文，再判断输出限制。仅当调用方结构完整性谓词已经验证完整结果时允许提前完成。
- mewu.visual-annotations/1 增加图片间 connection：target + geometry.rect 为起点，destination.target + destination.geometry.rect 为终点，两端各自使用 normalized 空间、非空稳定句柄和附件索引。先逐项验证结构，再独立解析两个句柄并纠偏索引；未知/重复句柄、同一图片、越界、视频或缺少覆盖层的附件一律拒绝该条而保留正文及合法兄弟项。最多展示 12 条；去重同时比较方向、两端目标和说明，不能误删指向不同图片的关系。
