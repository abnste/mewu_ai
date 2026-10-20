# Mewu 工具包

独立 Mewu API 插件，使用 `@mewu/sdk`，无需 Node、Electron 或模型密钥。

安装后在截图空间显示工具包。八个收藏格，右下角为垃圾桶。对象拖入收藏、拖出创建副本，叠放形成文件夹；收藏跨会话和 Agent 共用。停用或卸载插件会移除入口，保留收藏，重新安装同一插件后可继续使用。

图片、视频、HTML/SVG、文本、黑板及可拖动的绘制对象均通过宿主的受控对象库保存。不会把本机路径、会话消息或 API 密钥交给插件。HTML 保存源代码，不保存临时 JavaScript 堆。

需要 MewuAI 1.0.4 或更新版本，提供 `objects.library`、`ui.surfaces` 和 `objects.preview`。插件自己提供 `space-overlay` HTML 文档；宿主不内置工具包的布局或操作。旧版会明确提示更新。

图片收藏显示含标注的真实缩略图，拖动时预览跟随鼠标；文件夹显示其中图片的缩略图。其他对象仍可收藏与取出，暂不生成视频或交互 HTML 的缩略图。Esc 收起工具包，入口保留在截图空间原有的左下角工具条中。

打包使用 `node tools/extension-market/cli.mjs pack extensions/official/object-toolbox --file view.mjs --out object-toolbox.zip`。包只含 `mewu-plugin.json`、`index.mjs` 和 `view.mjs`。许可证 MPL-2.0。
