// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private string _archiveSessionId=Guid.NewGuid().ToString("N");
    private string _archiveSessionTitle=string.Empty;

    private void RestoreConversationArchive(ConversationSessionArchive? session)
    {
        if(session is null||_closed||_request is not null)return;
        var channel=_conversationChannels.FirstOrDefault(c=>ConversationArchivePolicy.Matches(session,c,_host.Settings));
        var entries=ConversationArchivePolicy.Entries(session);
        if(channel is null||entries.Count==0)return;
        // Hermes retains a remote session; a local history switch must not
        // silently continue the previous remote conversation.
        if(channel.Kind==ConversationChannelKind.Hermes)
        {
            var provider=_host.CreateConversationProvider(HermesConversationKind.Screen,channel.Id,out _);
            if(provider is not IConversationSessionReset reset||!reset.TryResetSession())
            {
                PromptStatus.Text=L("当前会话正忙，暂时无法打开历史。","The conversation is busy. Try opening history again later.");
                return;
            }
        }
        ++_historyLoadVersion;
        _selectedConversationChannelId=channel.Id;
        _host.RememberConversationChannel(channel.Id);
        _archiveSessionId=string.IsNullOrWhiteSpace(session.Id)?Guid.NewGuid().ToString("N"):session.Id;
        _archiveSessionTitle=session.Title;
        _history.Clear();
        MergeHistoryEntries(entries);
        _persistedHistory=entries;
        _lastSubmittedPrompt=string.Empty;_lastSubmittedTurnRecorded=false;
        ResetConversationProgress();
        AnswerText.Markdown=string.Empty;
        ResponseScroll.Visibility=Visibility.Collapsed;
        AnswerHeader.Visibility=AnswerScroll.Visibility=AnswerDivider.Visibility=Visibility.Collapsed;
        _reasoningBuffer.Clear();ReasoningText.Text=string.Empty;
        ReasoningToggle.Visibility=ReasoningPanel.Visibility=Visibility.Collapsed;
        UpdateChannelPickerItems();
        RefreshHistoryPreview();
        SetHistoryExpanded(true);
        PromptStatus.Text=L($"已打开历史会话：{session.Title}",$"Opened conversation: {session.Title}");
    }

    private async void OpenConversationArchive(object sender,RoutedEventArgs e)
    {
        e.Handled=true;
        if(_closed||_request is not null)return;
        try
        {
            var disk=_host.Settings.SaveConversationHistory?await new ConversationHistoryService().ReadRecentAsync(100):[];
            if(_closed||_request is not null)return;
            var sessions=ConversationHistoryService.CreateSessionArchive(disk.Concat(_host.GetAllSessionConversationHistory()),24);
            var menu=new ContextMenu();
            foreach(var session in sessions)
            {
                var item=new MenuItem{Header=session.Title,ToolTip=$"{session.Provider} · {session.Model}",IsEnabled=_host.CanOpenConversationSession(session)};
                item.Click+=(_,_)=>RestoreConversationArchive(session);
                menu.Items.Add(item);
            }
            if(menu.Items.Count==0)menu.Items.Add(new MenuItem{Header=L("暂无历史对话","No conversation history"),IsEnabled=false});
            menu.PlacementTarget=(UIElement)sender;menu.IsOpen=true;
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("ConversationArchiveLoad",ex);
            if(!_closed)PromptStatus.Text=L("暂时无法读取历史会话。","Conversation history is temporarily unavailable.");
        }
    }
}
