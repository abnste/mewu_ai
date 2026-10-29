# 悬浮交互文档运行契约

本契约适用于 Agent 生成的独立 HTML/SVG/Canvas 对象。插件界面另按插件的受控服务和存储协议运行。

- 每个对象是独立的沙箱文档，允许内联脚本、样式、DOM、SVG 和 Canvas。根元素使用 `data-mewu-object`；尺寸由用户要求和实际内容决定。仅输出一份最终完整代码，不同时输出草稿或修订稿。
- 文档没有同源身份。不能使用 `localStorage`、`sessionStorage`、IndexedDB、Cookie，也不能访问父窗口 DOM、宿主 IPC、本机文件或网络。连读取 `window.localStorage` 属性本身都可能抛出 `SecurityError`。普通计数和交互状态放在当前文档的闭包变量中；这不代表关闭、刷新后仍能保存状态。
- DOM 可用后，用明确的根元素选择器取得对象并绑定事件。脚本通常与根元素并列，不能用 `document.currentScript.closest(...)` 假设脚本是根元素的后代；异步回调中也不能依赖 `document.currentScript`。
- 先实现基本交互和可见反馈。音频、动画增强等可选 API 应检测支持情况并捕获失败，不能使初始化或点击处理整体中断。脚本、SVG 和样式必须完整、自包含；图片、字体、媒体仅使用文档自带的 `data:` 或 `blob:` 资源。
- 拖动、刷新和关闭由宿主提供。不要添加另一套外壳控件。需要跨重开保存内容时，只使用当前实际提供且获得授权的存储能力；不能编造宿主桥接 API 或声称内存变量已经持久化。
- 输出前核对初始化顺序、根节点查询、事件绑定和依赖。实际页面中的点击、计数或其他可见变化才是交互验证；代码语法通过或对象创建回执不等于交互已经实测。
- 最终答复中的完整 HTML/SVG 代码块由宿主直接呈现；这个交付路径不依赖本机文件、外部浏览器或新装的运行环境。没有实际提供页面测试工具时，进行静态核对后直接交付，不探测或安装 Node、Python、WebKit 等依赖，也不宣称已经通过页面点击测试。

依据：[HTML 沙箱规则](https://html.spec.whatwg.org/multipage/iframe-embed-object.html#attr-iframe-sandbox)、[Web Storage](https://developer.mozilla.org/en-US/docs/Web/API/Window/localStorage)、[currentScript 生命周期](https://developer.mozilla.org/en-US/docs/Web/API/Document/currentScript)。宿主使用 `sandbox allow-scripts`，不提供 `allow-same-origin`。
