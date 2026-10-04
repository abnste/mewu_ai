// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Windows;
using System.Windows.Input;
using mewu_ai_Assistant.Views;
using KeyEventArgs = System.Windows.Input.KeyEventArgs;
using KeyEventHandler = System.Windows.Input.KeyEventHandler;

/// <summary>
/// Passive, process-local diagnostics for the synthetic manual test only.
/// Records a fixed shortcut whitelist and focus metadata, never typed text.
/// Does not handle, suppress, create or inject input events.
/// </summary>
internal sealed class ManualDrawingInputObserver : IDisposable
{
    private readonly CaptureOverlayWindow _overlay;
    private readonly FrameworkElement _root;
    private readonly Action<string, object> _write;
    private readonly List<object> _events = [];
    private readonly PreProcessInputEventHandler _pre;
    private readonly ProcessInputEventHandler _post;
    private readonly KeyEventHandler _preview;
    private string? _lastState;
    private bool _disposed;
    internal string? FailureType { get; private set; }

    internal ManualDrawingInputObserver(CaptureOverlayWindow overlay, Action<string, object> write)
    {
        _overlay = overlay;
        _root = (FrameworkElement)overlay.FindName("Root");
        _write = write;
        _pre = (_, args) => Observe("preprocess", args.StagingItem.Input);
        _post = (_, args) => Observe("postprocess", args.StagingItem.Input);
        _preview = (_, args) => Observe("overlay-preview", args);
        InputManager.Current.PreProcessInput += _pre;
        InputManager.Current.PostProcessInput += _post;
        overlay.AddHandler(Keyboard.PreviewKeyDownEvent, _preview, true);
    }

    private object FocusState()
    {
        var focus = Keyboard.FocusedElement as DependencyObject;
        var source = PresentationSource.FromVisual(_overlay);
        var matchesSource = focus is not null && source is not null &&
            ReferenceEquals(PresentationSource.FromDependencyObject(focus), source);
        return new
        {
            keyboardFocusPresent = focus is not null,
            focusType = focus?.GetType().FullName,
            focusBelongsToOverlay = focus is not null && matchesSource && ReferenceEquals(Window.GetWindow(focus), _overlay),
            focusPresentationMatches = matchesSource,
            focusVisible = focus is UIElement { IsVisible: true },
            focusEnabled = focus is UIElement { IsEnabled: true } || focus is ContentElement { IsEnabled: true },
            focusFocusable = focus is UIElement { Focusable: true } || focus is ContentElement { Focusable: true },
            overlayActive = _overlay.IsActive,
            overlayHasKeyboardFocusWithin = _overlay.IsKeyboardFocusWithin,
            rootFocusable = _root.Focusable,
            rootKeyboardFocused = _root.IsKeyboardFocused,
            logicalFocusIsRoot = ReferenceEquals(FocusManager.GetFocusedElement(_overlay), _root),
            imeEnabledForFocus = focus is not null && InputMethod.GetIsInputMethodEnabled(focus),
            imeOn = InputMethod.Current.ImeState == InputMethodState.On,
            controlDown = Keyboard.Modifiers.HasFlag(ModifierKeys.Control),
            shiftDown = Keyboard.Modifiers.HasFlag(ModifierKeys.Shift),
            altDown = Keyboard.Modifiers.HasFlag(ModifierKeys.Alt)
        };
    }

    internal void UpdateState()
    {
        if (_disposed || FailureType is not null) return;
        try
        {
            var state = FocusState();
            var serialized = JsonSerializer.Serialize(state);
            if (serialized == _lastState) return;
            _write("input-focus-state.json", new
            {
                utc = DateTimeOffset.UtcNow, state,
                passive = true, inputInjected = false, inputSuppressed = false, textRecorded = false
            });
            _lastState = serialized;
        }
        catch (Exception error) { FailureType = error.GetType().Name; }
    }

    private void Observe(string stage, InputEventArgs input)
    {
        if (_disposed || FailureType is not null || _events.Count >= 256 || input is not KeyEventArgs key ||
            key.RoutedEvent != Keyboard.PreviewKeyDownEvent && key.RoutedEvent != Keyboard.KeyDownEvent) return;
        var original = key.Key == Key.ImeProcessed ? key.ImeProcessedKey : key.Key == Key.System ? key.SystemKey : key.Key;
        if (original is not (Key.C or Key.S or Key.P or Key.D or Key.Escape or Key.Enter or Key.Z or Key.Delete)) return;
        try
        {
            _events.Add(new
            {
                utc = DateTimeOffset.UtcNow, stage, key = original.ToString(),
                isPreview = key.RoutedEvent == Keyboard.PreviewKeyDownEvent,
                isImeProcessed = key.Key == Key.ImeProcessed, isSystemKey = key.Key == Key.System,
                handled = key.Handled, state = FocusState()
            });
            _write("input-key-routes.json", new
            {
                events = _events, passive = true, inputInjected = false,
                inputSuppressed = false, textRecorded = false, maximumEvents = 256
            });
        }
        catch (Exception error) { FailureType = error.GetType().Name; }
    }

    public void Dispose()
    {
        if (_disposed) return;
        UpdateState();
        _disposed = true;
        InputManager.Current.PreProcessInput -= _pre;
        InputManager.Current.PostProcessInput -= _post;
        _overlay.RemoveHandler(Keyboard.PreviewKeyDownEvent, _preview);
    }
}
