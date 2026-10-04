// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using mewu_ai_Assistant.Views;
using KeyEventArgs = System.Windows.Input.KeyEventArgs;
using Point = System.Windows.Point;
using TextBox = System.Windows.Controls.TextBox;

internal static class DrawingNumbersReplay
{
    internal static void Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.Static | BindingFlags.NonPublic | BindingFlags.Public;
        object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        var type = item.GetType();
        T Property<T>(object value, string name) => (T)value.GetType().GetProperty(name)!.GetValue(value)!;
        var elements = Property<IList>(item, "DrawingElements"); var order = Property<IList>(item, "DrawingOrder");
        var canvas = Property<InkCanvas>(item, "Markup"); var value = (TextBox)overlay.FindName("DrawingNumberValue");
        var snapshot = Invoke("CaptureOverlaySnapshot")!;
        int Next() => Property<int>(item, "NextDrawingNumber");
        Guid Preference() => Property<Guid>(item, "DrawingNumberPreference");
        void Undo() => Invoke("DrawUndo", overlay, new RoutedEventArgs());
        void Redo() => Invoke("DrawRedo", overlay, new RoutedEventArgs());
        void Set(int next) => value.Text = next.ToString(System.Globalization.CultureInfo.InvariantCulture);
        void Place(double x) => Invoke("BeginDrawingGesture", item, new Point(x, 70), canvas, 1, canvas);
        int Number(object element) => Property<int>(element, "Number");
        try
        {
            Invoke("DrawClear", overlay, new RoutedEventArgs());
            Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Number"));
            Set(12); var manual = Preference(); Place(60); Place(130);
            require(elements.Count == 2 && Number(elements[0]!) == 12 && Number(elements[1]!) == 13 && Next() == 14,
                "manual next12 places12/13 and advances to14");
            require(Preference() == manual, "automatic increments preserve preference identity");
            Undo(); require(elements.Count == 1 && Next() == 13, "number undo restores automatic next");
            Redo(); require(elements.Count == 2 && Next() == 14, "number redo restores automatic next");
            Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Select"));
            Invoke("BeginDrawingObjectSelection", item, new Point(60, 70), canvas); Invoke("CommitSelectedDrawingMove"); canvas.ReleaseMouseCapture();
            require((bool)Invoke("DeleteSelectedDrawingObject")! && elements.Count == 1, "selected number delete removes only selected number");
            Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Number"));
            Set(12); Place(200); require(Number(elements[1]!) == 12 && Next() == 13, "manual12 after deletion places12 without max-existing override");
            Set(7); Set(13); var changed = Preference(); Undo();
            require(Next() == 13 && Preference() == changed, "undo cannot overwrite manual7 then original13");
            Set(7); Set(12); changed = Preference(); Redo();
            require(Next() == 12 && Preference() == changed, "redo cannot overwrite manual7 then original12");
            var saved = Invoke("CaptureManualDrawingSnapshot", item)!;
            Set(78); Invoke("ApplyManualDrawingSnapshot", item, saved);
            require(Next() == 12 && Preference() == changed, "manual snapshot restores both next and preference identity");
            Invoke("DrawClear", overlay, new RoutedEventArgs());
            require(elements.Count == 0 && Next() == 1 && Preference() != changed, "clear resets next and preference identity");
            Undo(); require(elements.Count == 2 && Next() == 12 && Preference() == changed, "clear undo restores next and preference identity");
            var savedOuter = Invoke("CaptureOverlaySnapshot")!; Set(51); Invoke("ApplyOverlaySnapshot", savedOuter);
            require(Next() == 12 && Preference() == changed, "outer snapshot restores number preference");
            Invoke("UpdateDrawingContextControls", item); overlay.UpdateLayout(); value.Focus(); Keyboard.Focus(value);
            require(value.IsKeyboardFocused, "number input owns actual WPF keyboard focus");
            var count = elements.Count; var actions = order.Count;
            foreach (var key in new[] { Key.Delete, Key.Z })
            {
                var args = new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(overlay)!, Environment.TickCount, key)
                    { RoutedEvent = Keyboard.PreviewKeyDownEvent };
                value.RaiseEvent(args);
                require(!args.Handled && elements.Count == count && order.Count == actions, "number input " + key + " tunnels without annotation editing");
            }
            value.IsUndoEnabled = false; value.IsUndoEnabled = true; value.SelectAll(); EditingCommands.Delete.Execute(null, value);
            require(value.Text.Length == 0 && elements.Count == count, "native text Delete edits only number input");
            ApplicationCommands.Undo.Execute(null, value);
            require(value.Text == "12" && elements.Count == count && order.Count == actions, "native Undo command restores number text without annotation undo");
            var validNext = Next(); var validPreference = Preference();
            value.Text = "not-a-number";
            Keyboard.Focus((IInputElement)overlay.FindName("Root"));
            require(!value.IsKeyboardFocused && value.Text == "12" && Next() == validNext && Preference() == validPreference,
                "invalid number text normalizes on actual focus loss without changing next or preference");
        }
        finally { Invoke("ApplyOverlaySnapshot", snapshot); Invoke("SetDrawTool", Enum.Parse(typeof(CaptureOverlayWindow).GetNestedType("DrawTool", BindingFlags.NonPublic)!, "Select")); }
    }
}
