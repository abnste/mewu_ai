// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private static InkCanvas CreateDrawingMarkupCanvas()
    {
        var canvas = new InkCanvas { Background = Brushes.Transparent, IsHitTestVisible = false, ClipToBounds = true, Focusable = true };
        InputMethod.SetIsInputMethodEnabled(canvas, false);
        return canvas;
    }

    private void ConfigureOverlayInputMethods()
    {
        // Canvas does not inherit IsInputMethodEnabled into another focus
        // target. Configure each non-text surface explicitly; never change
        // the user's IME conversion mode or preferred input language.
        InputMethod.SetIsInputMethodEnabled(Root, false);
        InputMethod.SetIsInputMethodEnabled(QuickPrompt, true);
        InputMethod.SetIsInputMethodEnabled(DrawingFontFamily, true);
        InputMethod.SetIsInputMethodEnabled(DrawingFontSize, true);
        InputMethod.SetIsInputMethodEnabled(DrawingStrokeWidth, true);
        InputMethod.SetIsInputMethodEnabled(DrawingNumberValue, false);
    }

    // Keep the identity on the editor itself, including after an undo detaches
    // its old visual. A stale editor must not be mistaken for the prompt.
    private static readonly DependencyProperty IsDrawingTextEditorProperty =
        DependencyProperty.RegisterAttached("IsDrawingTextEditor", typeof(bool),
            typeof(CaptureOverlayWindow), new PropertyMetadata(false));

    private static bool IsDrawingTextEditor(TextBoxBase editor) =>
        (bool)editor.GetValue(IsDrawingTextEditorProperty);

    private bool IsActiveDrawingTextEditor(TextBoxBase editor) =>
        IsDrawingTextEditor(editor) && _drawingMode && _drawTool == DrawTool.Text &&
        editor is TextBox { Tag: Guid id, IsReadOnly: false } && id == _selectedDrawingElementId &&
        Active is { } item && ReferenceEquals(FindDrawingElementVisual(item, id), editor) &&
        editor.Focusable && editor.IsEnabled;

    private bool PreserveNativeTextInput(TextBoxBase editor) =>
        !IsDrawingTextEditor(editor) || IsActiveDrawingTextEditor(editor);

    private void ReleaseDrawingTextFocus(bool inactiveOnly)
    {
        if (Keyboard.FocusedElement is not TextBoxBase editor || !IsDrawingTextEditor(editor) ||
            inactiveOnly && IsActiveDrawingTextEditor(editor)) return;
        // Clearing keyboard focus alone leaves WPF's logical focus pointing at
        // the removed TextBox, which can be restored when its scope reactivates.
        FocusManager.SetFocusedElement(FocusManager.GetFocusScope(editor), null);
        Keyboard.ClearFocus();
        FocusManager.SetFocusedElement(this, Root);
        if (IsActive && !_closed) Keyboard.Focus(Root);
    }

    private void RestoreOverlayKeyboardRoute()
    {
        if (_closed || !IsActive || _drawingModalOpen || _systemFileDialogDepth > 0 ||
            ChannelPickerPopup.IsOpen || DrawingFontFamily.IsDropDownOpen ||
            DrawingFontSize.IsDropDownOpen || DrawingStrokeWidth.IsDropDownOpen) return;
        var focus = Keyboard.FocusedElement as DependencyObject;
        var source = PresentationSource.FromVisual(this);
        var connected = focus is not null && source is not null &&
            ReferenceEquals(PresentationSource.FromDependencyObject(focus), source);
        var usable = focus is UIElement { IsVisible: true, IsEnabled: true, Focusable: true } ||
            focus is ContentElement { IsEnabled: true, Focusable: true };
        if (connected && usable && (focus is not TextBoxBase editor || PreserveNativeTextInput(editor))) return;
        // Rebuilding a selection may remove the keyboard target even when it
        // was an InkCanvas/handle rather than a TextBox. Without a connected
        // target the next key never tunnels through the window's handler.
        FocusManager.SetFocusedElement(this, Root);
        Keyboard.Focus(Root);
    }

    // An old TextBox must never take a new pen stroke or shape gesture. Only
    // the active text editor receives text selection/caret input.
    private void UpdateDrawingTextInput(SelectionItem item)
    {
        foreach (var editor in item.Markup.Children.OfType<TextBox>())
        {
            var editable = _drawingMode && _drawTool == DrawTool.Text && ReferenceEquals(item, Active) &&
                editor.Tag is Guid id && id == _selectedDrawingElementId;
            InputMethod.SetIsInputMethodEnabled(editor, editable);
            editor.IsHitTestVisible = editor.Focusable = editable;
            if (!editable) editor.BorderBrush = Brushes.Transparent;
        }
        ReleaseDrawingTextFocus(inactiveOnly: true);
    }
}
