# 研究便笺

这是与对象工具箱业务不同的独立插件：可编辑便笺，加上拖入的参考对象列表。
界面、编辑规则、列表操作和状态格式全部在本包的 `view.mjs` 与 `index.mjs`。
宿主不提供便笺专用接口。

需要支持 `ui.surfaces` 与 `objects.library` 的开发版 Mewu。正文点击保存后持久化；
拖入引用、移除引用分别提交对象事务。停用或卸载释放界面，重装同一插件可恢复数据。

在仓库根目录打包：

```sh
node tools/extension-market/cli.mjs pack extensions/examples/research-notes --file view.mjs --out research-notes.zip
```

包只含清单、入口和视图模块，未发布到社区市场。MPL-2.0。
