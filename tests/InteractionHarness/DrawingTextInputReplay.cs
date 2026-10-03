// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using mewu_ai_Assistant.Views;
using KeyEventArgs = System.Windows.Input.KeyEventArgs;
using Point = System.Windows.Point;
using TextBox = System.Windows.Controls.TextBox;
using RichTextBox = System.Windows.Controls.RichTextBox;
using ComboBox = System.Windows.Controls.ComboBox;

/// <summary>Local WPF focus/restore replay, without desktop keys or exports.</summary>
internal static class DrawingTextInputReplay
{
    internal static async Task Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public | BindingFlags.DeclaredOnly;
        object? Invoke(string name, params object?[] args) =>
            typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        void RequireConnectedKeyboardFocus(string description)
        {
            var focus = Keyboard.FocusedElement as DependencyObject;
            require(focus is not null && ReferenceEquals(PresentationSource.FromDependencyObject(focus),
                PresentationSource.FromVisual(overlay)) && ReferenceEquals(Window.GetWindow(focus), overlay), description);
        }
        void RequireShortcutImeDisabled(string description)
        {
            require(Keyboard.FocusedElement is DependencyObject focus && !InputMethod.GetIsInputMethodEnabled(focus), description);
        }
        void ComposeChinese(TextBox target, string description)
        {
            var previous = target.Text;
            try
            {
                target.Select(target.Text.Length, 0);
                target.RaiseEvent(new TextCompositionEventArgs(Keyboard.PrimaryDevice,
                    new TextComposition(InputManager.Current, target, "中文回放"))
                    { RoutedEvent = TextCompositionManager.TextInputEvent });
                require(target.Text == previous + "中文回放", description);
            }
            finally { target.Text = previous; }
        }
        var itemType = item.GetType();
        var canvas = (InkCanvas)itemType.GetProperty("Markup")!.GetValue(item)!;
        var root = (Canvas)overlay.FindName("Root");
        var editor = canvas.Children.OfType<TextBox>().First(text => !string.IsNullOrEmpty(text.Text));
        var id = (Guid)editor.Tag;
        var words = editor.Text;
        var point = new Point(InkCanvas.GetLeft(editor) + 8, InkCanvas.GetTop(editor) + 8);
        Invoke("TryEditDrawingText", item, point, canvas);
        await overlay.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        require((bool)Invoke("IsActiveDrawingTextEditor", editor)! && (bool)Invoke("PreserveNativeTextInput", editor)!,
            "the explicitly reopened annotation retains native text input and undo eligibility");
        require((bool)Invoke("PreserveNativeTextInput", overlay.FindName("QuickPrompt"))! &&
            (bool)Invoke("PreserveNativeTextInput", new RichTextBox { IsReadOnly = true })!,
            "prompt and read-only answer text retain their existing native key handling");
        require(!InputMethod.GetIsInputMethodEnabled(root) && !InputMethod.GetIsInputMethodEnabled(canvas),
            "non-text root and ink canvas disable IME interception for their bare-letter shortcuts");
        require(editor.IsKeyboardFocused && InputMethod.GetIsInputMethodEnabled(editor),
            "the actual focused annotation editor independently permits IME while its canvas disables it");
        var prompt = (TextBox)overlay.FindName("QuickPrompt");
        require(InputMethod.GetIsInputMethodEnabled(prompt), "the prompt keeps IME enabled for Chinese text input");
        foreach (var name in new[] { "DrawingFontFamily", "DrawingFontSize" })
        {
            var combo = (ComboBox)overlay.FindName(name); combo.ApplyTemplate();
            require(InputMethod.GetIsInputMethodEnabled(combo), name + " keeps its text-entry IME policy enabled");
            if (combo.IsEditable)
                require(combo.Template.FindName("PART_EditableTextBox", combo) is TextBox text && InputMethod.GetIsInputMethodEnabled(text),
                    name + " editable template permits its own text-entry IME handling");
        }
        // This uses WPF's routed text-composition path only. It does not toggle
        // the OS input method or replace the separate physical-IME test.
        ComposeChinese(editor, "focused annotation accepts routed Chinese composition without losing native text input");
        ComposeChinese(prompt, "prompt accepts routed Chinese composition while the screenshot canvas disables IME");

        // Exit directly, without DrawDone's clipboard operation.
        Invoke("ExitDrawingMode");
        require(Field("_drawingMode") is false && !editor.Focusable &&
            editor.ReadLocalValue(UIElement.IsHitTestVisibleProperty) is false &&
            !InputMethod.GetIsInputMethodEnabled(editor) && !(bool)Invoke("PreserveNativeTextInput", editor)!,
            "leaving drawing makes its old annotation editor inert for capture shortcuts");
        require(ReferenceEquals(FocusManager.GetFocusedElement(overlay), root) && !editor.IsKeyboardFocusWithin,
            "drawing exit replaces annotation logical focus with the overlay root");
        RequireConnectedKeyboardFocus("drawing exit leaves a real keyboard target connected to this overlay");
        RequireShortcutImeDisabled("drawing exit focuses a target whose bare-letter shortcuts bypass IME");

        var originalBounds = (Rect)itemType.GetField("Bounds")!.GetValue(item)!;
        var beforeMove = Invoke("CaptureOverlaySnapshot")!;
        var movedBounds = new Rect(originalBounds.X + 12, originalBounds.Y + 8, originalBounds.Width, originalBounds.Height);
        Invoke("SetSelectionBoundsPreservingManualContent", item, movedBounds);
        Invoke("UpdateSelection", item);
        Invoke("RecordGeometryOperationIfChanged", beforeMove, "focus replay move");
        var beforeResize = Invoke("CaptureOverlaySnapshot")!;
        Invoke("SetSelectionBoundsPreservingManualContent", item,
            new Rect(movedBounds.X, movedBounds.Y, movedBounds.Width + 20, movedBounds.Height + 20));
        Invoke("UpdateSelection", item);
        Invoke("RecordGeometryOperationIfChanged", beforeResize, "focus replay resize");

        // Model an old editor that retained focus despite having left drawing.
        // This changes only the isolated replay control, never global input.
        editor.Focusable = true;
        Keyboard.Focus(editor);
        require(editor.IsKeyboardFocused, "focus regression establishes stale annotation focus before snapshot restore");
        Invoke("UndoOverlayOperation");
        await overlay.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        var restored = canvas.Children.OfType<TextBox>().Single(text => Equals(text.Tag, id));
        require(!ReferenceEquals(editor, restored) && restored.Text == words &&
            Equals(itemType.GetField("Bounds")!.GetValue(item), movedBounds),
            "geometry undo restores a new editor and the pre-resize bounds without changing its wording");
        require(!restored.Focusable && restored.ReadLocalValue(UIElement.IsHitTestVisibleProperty) is false &&
            !InputMethod.GetIsInputMethodEnabled(restored) && !InputMethod.GetIsInputMethodEnabled(editor) &&
            !(bool)Invoke("PreserveNativeTextInput", restored)! && !(bool)Invoke("PreserveNativeTextInput", editor)!,
            "both restored and detached annotation editors cannot block native capture shortcuts outside drawing");
        require(ReferenceEquals(FocusManager.GetFocusedElement(overlay), root) && !editor.IsKeyboardFocusWithin &&
            !restored.IsKeyboardFocusWithin, "snapshot restore clears stale keyboard and logical annotation focus");
        RequireConnectedKeyboardFocus("geometry undo retains a live keyboard route after dispatcher focus reevaluation");
        RequireShortcutImeDisabled("geometry undo restores non-text focus with IME interception disabled");

        // Also cover a missing non-text target. The old text-only cleanup did
        // nothing here, so the next physical key could have no window route.
        Keyboard.ClearFocus();
        FocusManager.SetFocusedElement(overlay, null);
        require(Keyboard.FocusedElement is null, "focus regression establishes the missing keyboard target case");
        Invoke("ApplyOverlaySnapshot", beforeResize);
        await overlay.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        require(ReferenceEquals(Keyboard.FocusedElement, root),
            "snapshot restore repairs an absent keyboard target with the overlay root");
        RequireConnectedKeyboardFocus("repaired root focus belongs to the live overlay presentation source");
        RequireShortcutImeDisabled("repairing a missing keyboard target preserves the root's disabled IME policy");
        restored = canvas.Children.OfType<TextBox>().Single(text => Equals(text.Tag, id));

        // D uses the same TextBox shortcut gate as C/S/P, with no clipboard,
        // save dialog or pin side effects. Simulate the stale state once more.
        restored.Focusable = true;
        Keyboard.Focus(restored);
        require(restored.IsKeyboardFocused, "shortcut regression establishes inactive editor keyboard focus");
        var key = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(overlay)!, Environment.TickCount, Key.D)
        { RoutedEvent = Keyboard.PreviewKeyDownEvent };
        RequireConnectedKeyboardFocus("shortcut replay starts at an actual connected keyboard target");
        var keyboardTarget = Keyboard.FocusedElement as UIElement ??
            throw new InvalidOperationException("The routed shortcut requires a live UIElement keyboard target.");
        keyboardTarget.RaiseEvent(key);
        require(key.Handled && Field("_drawingMode") is true && !restored.IsKeyboardFocusWithin &&
            !(bool)Invoke("PreserveNativeTextInput", restored)! && restored.Text == words,
            "inactive annotation focus cannot swallow the shared capture shortcut path or receive typed text");
        RequireConnectedKeyboardFocus("routed shortcut leaves keyboard focus connected to the overlay");
        RequireShortcutImeDisabled("returning to the drawing canvas keeps non-text IME interception disabled");
        Invoke("TryEditDrawingText", item, new Point(InkCanvas.GetLeft(restored) + 8, InkCanvas.GetTop(restored) + 8), canvas);
        await overlay.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        require(restored.IsKeyboardFocused && InputMethod.GetIsInputMethodEnabled(restored) &&
            (bool)Invoke("IsActiveDrawingTextEditor", restored)!,
            "reopening restored annotation text re-enables IME only for its actual active editor");
    }
}
