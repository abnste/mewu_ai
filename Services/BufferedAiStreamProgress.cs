using System.Runtime.ExceptionServices;
using System.Text;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

/// <summary>
/// Batches normalized Provider deltas before they reach the UI queue. The first
/// batch is posted immediately; subsequent batches yield to input and rendering.
/// One instance belongs to exactly one request and must be disposed on its UI thread.
/// </summary>
internal sealed class BufferedAiStreamProgress : IProgress<AiStreamDelta>, IDisposable
{
    internal static readonly TimeSpan RefreshInterval = TimeSpan.FromMilliseconds(80);
    private readonly object _gate = new();
    private readonly StringBuilder _content = new();
    private readonly StringBuilder _reasoning = new();
    private readonly Dispatcher _dispatcher;
    private readonly DispatcherTimer _timer;
    private readonly Func<bool> _canAccept;
    private readonly Action<AiStreamDelta> _render;
    private bool _started;
    private bool _stopped;
    private ExceptionDispatchInfo? _failure;

    internal BufferedAiStreamProgress(Dispatcher dispatcher, Func<bool> canAccept, Action<AiStreamDelta> render)
    {
        dispatcher.VerifyAccess();
        _dispatcher = dispatcher;
        _canAccept = canAccept;
        _render = render;
        _timer = new DispatcherTimer(DispatcherPriority.Background, dispatcher) { Interval = RefreshInterval };
        _timer.Tick += OnTick;
    }

    public void Report(AiStreamDelta value)
    {
        lock (_gate)
        {
            if (_stopped || (value.Content.Length == 0 && value.ReasoningContent.Length == 0)) return;
            _content.Append(value.Content);
            _reasoning.Append(value.ReasoningContent);
            if (_started) return;
            _started = true;
            // Posting under the lock prevents disposal from racing the initial wakeup.
            _dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() =>
            {
                Flush();
                lock (_gate) { if (!_stopped) _timer.Start(); }
            }));
        }
    }

    private void OnTick(object? sender, EventArgs e) => Flush();

    internal void Flush()
    {
        _dispatcher.VerifyAccess();
        lock (_gate) { if (_stopped) return; }
        if (!_canAccept()) { Dispose(); return; }
        AiStreamDelta batch;
        lock (_gate)
        {
            if (_content.Length == 0 && _reasoning.Length == 0) return;
            batch = new(_content.ToString(), _reasoning.ToString());
            _content.Clear();
            _reasoning.Clear();
        }
        try { _render(batch); }
        catch (Exception error)
        {
            // Surface the error in the request's normal failure path, rather
            // than crashing the application from a Dispatcher callback.
            _failure = ExceptionDispatchInfo.Capture(error);
            Dispose();
        }
    }

    internal void ThrowIfFaulted() => _failure?.Throw();

    public void Dispose()
    {
        _dispatcher.VerifyAccess();
        lock (_gate)
        {
            _stopped = true;
            _content.Clear();
            _reasoning.Clear();
        }
        _timer.Stop();
        _timer.Tick -= OnTick;
    }
}
