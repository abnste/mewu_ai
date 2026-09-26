// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using mewu_ai_Assistant.Services;
namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    // MCP 常驻动作：圈选完成后，工具栏上的 钉钉 / 飞书 / Obsidian 按钮把
    // 当前活动选区渲染为 PNG 后发送/存档。所有动作走 BeginOverlayOperation
    // 模式（Esc 可取消、期间工具栏禁用），结果以 PromptStatus 反馈。

    /// <summary>外部动作（打开网站、发出邮件）完成后收起识屏浮层：
    /// 全屏冻结帧不再遮挡新打开的窗口，也避免后续交互卡顿。
    /// AI 应答进行中时不关闭（Close 会取消在途请求），只收起工具栏。</summary>
    private void DismissOverlayAfterExternalAction()
    {
        if(_closed)return;
        if(_overlayRequest is null){Close();return;}
        Toolbar.Visibility=Visibility.Collapsed;
        HideScreenEntityBar();
        SetPromptBarHidden(true);
    }

    private void SendToDingTalk(object sender,RoutedEventArgs e)=>_=SendSelectionImageAsync(
        L("正在发送到钉钉…按 Esc 可取消","Sending to DingTalk… press Esc to cancel"),
        async(png,token)=>await DingTalkService.SendImageAsync(_host.Settings,png,token).ConfigureAwait(false));

    private void SendToFeishu(object sender,RoutedEventArgs e)=>_=SendSelectionImageAsync(
        L("正在发送到飞书…按 Esc 可取消","Sending to Feishu… press Esc to cancel"),
        async(png,token)=>await FeishuService.SendImageAsync(_host.Settings,png,token).ConfigureAwait(false));

    private void SaveToObsidian(object sender,RoutedEventArgs e)=>_=SendSelectionImageAsync(
        L("正在保存到 Obsidian…","Saving to Obsidian…"),
        async(png,token)=>await ObsidianVaultService.SaveNoteAsync(_host.Settings,png,token).ConfigureAwait(false),
        L("已存入 Obsidian：","Saved to Obsidian: "));

    private void SaveToIma(object sender,RoutedEventArgs e)=>_=SendSelectionImageAsync(
        L("正在归档到 ima 知识库…","Archiving to ima…"),
        async(png,token)=>await ImaVaultService.SaveImageAsync(_host.Settings,png,token).ConfigureAwait(false));

    private async Task SendSelectionImageAsync(string busyStatus,Func<byte[],CancellationToken,Task<string>> action,string? successLabel=null)
    {
        if(RejectIfOverlayOperationBusy()||Active is not { } item)return;
        if(item.VideoPath is not null)return; // 视频选区不支持发送（钉钉/飞书图片接口只收静态图）
        var operation=BeginOverlayOperation(busyStatus);
        byte[]? png=null;
        try
        {
            // 在 UI 线程完成渲染再进入后台，与贴图/保存路径一致。
            var image=RenderSelectionImage(item,true,true,true);
            png=ObsidianVaultService.EncodePng(image);
            var outcome=await Task.Run(()=>action(png,operation.Token),operation.Token).ConfigureAwait(true);
            if(IsOverlayOperationActive(operation,item))PromptStatus.Text=successLabel is null?outcome:$"{successLabel}{outcome}";
        }
        catch(OperationCanceledException){if(IsOverlayOperationActive(operation,item))PromptStatus.Text=L("已取消发送。","Send canceled.");}
        catch(Exception ex)
        {
            new PrivacyLogger().Error("McpShareImage",ex);
            if(IsOverlayOperationActive(operation,item))PromptStatus.Text=$"{L("发送失败","Send failed")}：{ex.Message}";
        }
        finally{if(png is not null)System.Security.Cryptography.CryptographicOperations.ZeroMemory(png);EndOverlayOperation(operation);}
    }
}
