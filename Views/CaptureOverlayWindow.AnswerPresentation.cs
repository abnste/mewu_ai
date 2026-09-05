using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private bool _followAnswerTail = true;
    private bool _answerLayoutQueued;
    private double _answerReadingOffset;

    private void RefreshAnswer(string markdown)
    {
        if (!_followAnswerTail && !_answerLayoutQueued) _answerReadingOffset = AnswerScroll.VerticalOffset;
        AnswerText.Markdown = markdown;
        QueueAnswerLayout();
    }

    private void QueueAnswerLayout()
    {
        if (_answerLayoutQueued || _closed) return;
        _answerLayoutQueued = true;
        _ = Dispatcher.BeginInvoke(DispatcherPriority.Loaded, new Action(() =>
        {
            if (_closed) { _answerLayoutQueued = false; return; }
            // Let WPF perform its regular layout pass. Forcing UpdateLayout
            // after every streamed token used to measure the entire card twice.
            PositionPromptBar();
            if (_followAnswerTail) AnswerScroll.ScrollToEnd();
            else AnswerScroll.ScrollToVerticalOffset(_answerReadingOffset);
            _answerLayoutQueued = false;
            LatestAnswerButton.Visibility = _followAnswerTail ? Visibility.Collapsed : Visibility.Visible;
        }));
    }

    private void AnswerScrollChanged(object sender, ScrollChangedEventArgs e)
    {
        if (!ReferenceEquals(e.OriginalSource, AnswerScroll) || _answerLayoutQueued ||
            e.ExtentHeightChange != 0 || e.ViewportHeightChange != 0 || e.VerticalChange == 0) return;
        _answerReadingOffset = AnswerScroll.VerticalOffset;
        _followAnswerTail = AnswerScroll.ScrollableHeight - AnswerScroll.VerticalOffset <= 24;
        LatestAnswerButton.Visibility = _followAnswerTail ? Visibility.Collapsed : Visibility.Visible;
    }

    private void AnswerPreviewMouseWheel(object sender, MouseWheelEventArgs e)
    {
        if (e.Delta <= 0 || AnswerScroll.ScrollableHeight <= 0) return;
        _followAnswerTail = false;
        _answerReadingOffset = AnswerScroll.VerticalOffset;
        LatestAnswerButton.Visibility = Visibility.Visible;
    }

    private void JumpToLatestAnswer(object sender, RoutedEventArgs e)
    {
        _followAnswerTail = true;
        LatestAnswerButton.Visibility = Visibility.Collapsed;
        QueueAnswerLayout();
    }
}
