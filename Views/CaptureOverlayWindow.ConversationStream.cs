// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private StackPanel? _conversationStream;
    private FrameworkElement? _liveConversationContent;
    private bool UnifiedConversation=>_conversationStream is not null;

    private void UpdateConversationStream()
    {
        var expanded=_historyExpanded;
        if(expanded==UnifiedConversation)return;
        if(expanded)
        {
            _liveConversationContent=(FrameworkElement)ResponseScroll.Content;
            ResponseScroll.Content=null;
            HistoryScroll.Content=null;
            _conversationStream=new StackPanel();
            _conversationStream.Children.Add(HistoryItems);
            _conversationStream.Children.Add(_liveConversationContent);
            HistoryScroll.Content=_conversationStream;
            AnswerText.VerticalScrollBarVisibility=ScrollBarVisibility.Disabled;
            AnswerScroll.MaxHeight=double.PositiveInfinity;
            HistoryScroll.PreviewMouseWheel+=ConversationStreamWheel;
            HistoryScroll.ScrollChanged+=ConversationStreamChanged;
        }
        else
        {
            HistoryScroll.PreviewMouseWheel-=ConversationStreamWheel;
            HistoryScroll.ScrollChanged-=ConversationStreamChanged;
            _conversationStream!.Children.Clear();
            HistoryScroll.Content=HistoryItems;
            ResponseScroll.Content=_liveConversationContent;
            _conversationStream=null;_liveConversationContent=null;
            AnswerText.VerticalScrollBarVisibility=ScrollBarVisibility.Auto;
        }
        ApplyBubbleAnswerStyle(expanded);
    }

    private void ConversationStreamWheel(object sender,MouseWheelEventArgs e)
    {
        if(e.Delta>0)_followAnswerTail=false;
        HistoryScroll.ScrollToVerticalOffset(HistoryScroll.VerticalOffset-e.Delta/3.0);
        e.Handled=true;
    }

    private void ConversationStreamChanged(object sender,ScrollChangedEventArgs e)
    {
        if(!ReferenceEquals(e.OriginalSource,HistoryScroll))return;
        if(e.ExtentHeightChange!=0||e.ViewportHeightChange!=0)
        {
            if(_followAnswerTail)HistoryScroll.ScrollToEnd();
        }
        else if(e.VerticalChange!=0)_followAnswerTail=HistoryScroll.ScrollableHeight-HistoryScroll.VerticalOffset<=24;
    }
}
