// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Text;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private ConversationWidgetState _conversationProgressState;
    private string _conversationProgressPreview=string.Empty;
    private bool _conversationProgressHasReasoning;

    private void ResetConversationProgress()
    {
        _conversationProgressState=ConversationWidgetState.Idle;
        _conversationProgressPreview=string.Empty;
        _conversationProgressHasReasoning=false;
        RefreshConversationWidget();
    }

    private void BeginConversationProgress(CancellationTokenSource request)
    {
        if(!CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;
        ResetConversationProgress();
        _conversationProgressState=ConversationWidgetState.Thinking;
        RefreshConversationWidget();
    }

    private void UpdateConversationProgress(CancellationTokenSource request,string text,bool reasoning=false)
    {
        if(_conversationProgressState!=ConversationWidgetState.Thinking||
            !CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed)||
            (!reasoning&&_conversationProgressHasReasoning))return;
        var preview=GetConversationProgressPreview(text);
        if(preview.Length==0)return;
        _conversationProgressHasReasoning|=reasoning;
        _conversationProgressPreview=preview;
        RefreshConversationWidget();
    }

    private void FinishConversationProgress(CancellationTokenSource request,bool succeeded)
    {
        if(_closed||!ReferenceEquals(_request,request))return;
        _conversationProgressState=request.IsCancellationRequested?ConversationWidgetState.Canceled:
            succeeded?ConversationWidgetState.Completed:ConversationWidgetState.Failed;
        RefreshConversationWidget();
    }

    private void SetConversationFinalReasoning(CancellationTokenSource request,string reasoning)
    {
        if(_conversationProgressState!=ConversationWidgetState.Thinking||
            !CaptureOverlayPolicy.CanAcceptAiUpdate(_request,request,_closed))return;
        _conversationProgressHasReasoning=false;
        _conversationProgressPreview=string.Empty;
        UpdateConversationProgress(request,reasoning,true);
        RefreshConversationWidget();
    }

    private void RefreshConversationWidget()=>
        _conversationWidget?.UpdateProgress(_conversationProgressState,_conversationProgressPreview);

    private static string GetConversationProgressPreview(string text)
    {
        // Keep only a bounded recent excerpt; never copy the whole answer or
        // split a surrogate pair/combining sequence at the display boundary.
        var start=Math.Max(0,text.Length-512);
        if(start>0&&char.IsLowSurrogate(text[start]))start++;
        var tail=new StringBuilder(512);
        foreach(var character in text.AsSpan(start))
        {
            if(char.IsWhiteSpace(character))
            {
                if(tail.Length>0&&tail[^1]!=' ')tail.Append(' ');
            }
            else if(!char.IsControl(character))tail.Append(character);
        }
        var value=tail.ToString().Trim();
        var elements=StringInfo.ParseCombiningCharacters(value);
        return elements.Length>48?value[elements[^48]..]:value;
    }
}
