# 选区宿主动作示例（region-actions）

在截图选区工具条上注册两个命令，演示宿主动作门面 `ctx.actions.invoke`：

- **复制此选区**：`ctx.actions.invoke({ action: 'copy', regionId })`，宿主把当前选区（含标注）写入剪贴板。
- **贴图此选区**：`ctx.actions.invoke({ action: 'pin', regionId })`，宿主使用当前启用的贴图贡献把选区贴图为置顶窗口。

命令从选区工具条发起时宿主在 `context.regionId` 中传入当前选区；插件把它原样交给宿主动作。宿主重新校验该选区仍存在于当前场景、空间不在忙（保存中、录屏、长截图、退出准备），再执行与手动工具条完全相同的动作代码。`save`（原生保存对话框，取消视为未完成）和 `reference`（把选区加入当前对话引用，已存在则幂等成功）同样可用，只是未在本示例中注册命令。

动作失败（选区已关闭、宿主正忙、用户取消保存、没有可用贴图等）会以宿主调用诊断形式显示，例如 `action.invoke：已取消 [HOST_ACTION_FAILED]`；插件 30 秒内未等到宿主回复则显示 `action.invoke：动作执行超时 [HOST_ACTION_FAILED]`。

## 权限

四个动作按各自效果拆分权限，宿主按每次调用的 `action` 值分派：

| 动作 | 权限 | 语义 |
| --- | --- | --- |
| `copy` | `action.clipboard` | 剪贴板写入 |
| `save` | `action.export` | 原生保存对话框导出 |
| `pin` | `action.pin` | 创建置顶贴图 |
| `reference` | `action.reference` | 修改当前对话引用 |

本示例只声明 `action.clipboard` 与 `action.pin`。删除其中一个字段（例如去掉 `"action.pin"`）后重新打包安装，再执行“贴图此选区”，即可复现权限拒绝：宿主在调用前直接拒绝，显示 `action.invoke：插件未声明所需权限 [HOST_PERMISSION_DENIED]`，不会弹出任何贴图。

`variant: 'annotated' | 'original'` 仅对 `copy` 与 `save` 有效，默认 `annotated`；`copy` 始终复制含标注版本。从 Mewu 仓库根目录打包：

```powershell
node tools/extension-market/cli.mjs pack extensions/examples/region-actions --out D:/packages/region-actions-0.1.0.zip
```

示例仅使用宿主自带界面，不包含任何第三方素材。
