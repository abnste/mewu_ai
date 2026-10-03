// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Controls;
using System.Windows.Input;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private void UpdateShapeDrawingPreview(InkCanvas canvas,System.Windows.Point point,bool constrain)
    {
        if(_drawPreview is not null)canvas.Strokes.Remove(_drawPreview);
        _drawPreview=CreateShapeStroke(canvas,_drawStart,point,_drawTool,constrain);
        canvas.Strokes.Add(_drawPreview);
    }

    private bool RefreshDrawingConstraint(bool constrain)
    {
        if(!_drawingMode||Active is not { } item||!item.Markup.IsMouseCaptured)return false;
        var canvas=item.Markup;
        var point=Mouse.GetPosition(canvas);
        if(_drawingMoveOriginalElement is not null||_drawingMoveOriginalStroke is not null||_drawingMoveOriginalRegionMark is not null)
        {
            if(_drawingResizeHandle<0)return false;
            ResizeSelectedDrawingObjectWithConstraint(item,point,canvas,constrain);
            return true;
        }
        if(_drawTool is not (DrawTool.Line or DrawTool.Arrow or DrawTool.Rectangle or DrawTool.Ellipse))return false;
        if(_drawPreview is null&&(point-_drawStart).Length<.5)return false;
        UpdateShapeDrawingPreview(canvas,point,constrain);
        return true;
    }

    private void DrawingModifierKeyUp(object sender,KeyEventArgs e)
    {
        if(e.Key is not (Key.LeftShift or Key.RightShift))return;
        var otherShift=e.Key==Key.LeftShift?Key.RightShift:Key.LeftShift;
        if(RefreshDrawingConstraint(Keyboard.IsKeyDown(otherShift)))e.Handled=true;
    }
}
