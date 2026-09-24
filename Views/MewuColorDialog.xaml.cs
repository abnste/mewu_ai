// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

public partial class MewuColorDialog:Window
{
    private bool _updating;
    private HsvColor _hsv;
    private static readonly Lazy<BitmapSource> RingBitmap=new(CreateHueRing);
    internal Color SelectedColor { get; private set; }

    private MewuColorDialog(Color initial)
    {
        InitializeComponent();SelectedColor=initial;_hsv=ColorSpectrumMath.FromRgb(initial);HueRing.Source=RingBitmap.Value;
        SourceInitialized+=(_,_)=>{var handle=new WindowInteropHelper(this).Handle;NativeMethods.TryUseSystemRoundedCorners(handle);NativeMethods.ApplyOwnedWindowCaptureVisibility(handle,Owner is null?IntPtr.Zero:new WindowInteropHelper(Owner).Handle);};
        Loaded+=(_,_)=>SetChannels(initial.R,initial.G,initial.B);
        Closed+=(_,_)=>{if(HueRing.IsMouseCaptured)HueRing.ReleaseMouseCapture();if(SaturationValuePlane.IsMouseCaptured)SaturationValuePlane.ReleaseMouseCapture();};
    }

    internal static bool TryChoose(Window owner,Color initial,out Color selected,string? title=null)
    {
        var dialog=new MewuColorDialog(initial){Owner=owner,Topmost=owner.Topmost};if(title is not null)dialog.Title=title;var accepted=dialog.ShowDialog()==true;selected=accepted?dialog.SelectedColor:initial;return accepted;
    }

    private void ValueTextChanged(object sender,System.Windows.Controls.TextChangedEventArgs e)
    {
        if(_updating||!IsLoaded)return;
        if(byte.TryParse(RedValue.Text,NumberStyles.None,CultureInfo.InvariantCulture,out var red)&&
           byte.TryParse(GreenValue.Text,NumberStyles.None,CultureInfo.InvariantCulture,out var green)&&
           byte.TryParse(BlueValue.Text,NumberStyles.None,CultureInfo.InvariantCulture,out var blue))
        {
            SelectedColor=Color.FromRgb(red,green,blue);_hsv=ColorSpectrumMath.FromRgb(SelectedColor,_hsv.Hue);RefreshColor(writeRgb:false);
        }
        else SetInputValidity(false,LocalizationService.T("RGB 请输入 0–255 的整数。","Enter whole RGB values from 0 to 255."));
    }
    private void SetChannels(byte red,byte green,byte blue)
    {
        SelectedColor=Color.FromRgb(red,green,blue);_hsv=ColorSpectrumMath.FromRgb(SelectedColor,_hsv.Hue);RefreshColor();
    }

    private void HexTextChanged(object sender,TextChangedEventArgs e)
    {
        if(_updating||!IsLoaded)return;
        var hex=HexValue.Text.Trim();if(hex.StartsWith('#'))hex=hex[1..];
        if(hex.Length==6&&uint.TryParse(hex,NumberStyles.AllowHexSpecifier,CultureInfo.InvariantCulture,out var value))
        {
            SelectedColor=Color.FromRgb((byte)(value>>16),(byte)(value>>8),(byte)value);
            _hsv=ColorSpectrumMath.FromRgb(SelectedColor,_hsv.Hue);RefreshColor(writeHex:false);
        }
        else SetInputValidity(false,LocalizationService.T("颜色格式为 #RRGGBB。","Use the color format #RRGGBB."));
    }

    private void SetHue(double hue)
    {
        if(!double.IsFinite(hue))return;
        _hsv=_hsv with{Hue=(hue%360+360)%360};SelectedColor=ColorSpectrumMath.ToRgb(_hsv);RefreshColor();
    }

    private void SetSaturationValue(double saturation,double value)
    {
        if(!double.IsFinite(saturation)||!double.IsFinite(value))return;
        _hsv=_hsv with{Saturation=Math.Clamp(saturation,0,1),Value=Math.Clamp(value,0,1)};
        SelectedColor=ColorSpectrumMath.ToRgb(_hsv);RefreshColor();
    }

    private void RefreshColor(bool writeRgb=true,bool writeHex=true)
    {
        _updating=true;
        try
        {
            if(writeRgb){RedValue.Text=SelectedColor.R.ToString(CultureInfo.InvariantCulture);GreenValue.Text=SelectedColor.G.ToString(CultureInfo.InvariantCulture);BlueValue.Text=SelectedColor.B.ToString(CultureInfo.InvariantCulture);}
            if(writeHex)HexValue.Text=$"#{SelectedColor.R:X2}{SelectedColor.G:X2}{SelectedColor.B:X2}";
            ColorPreview.Background=new SolidColorBrush(SelectedColor);
            HueBase.Background=new SolidColorBrush(ColorSpectrumMath.ToRgb(new HsvColor(_hsv.Hue,1,1)));
            var angle=_hsv.Hue*Math.PI/180;
            Canvas.SetLeft(HueSelector,140+124*Math.Cos(angle)-7);Canvas.SetTop(HueSelector,140+124*Math.Sin(angle)-7);
            Canvas.SetLeft(SaturationValueSelector,62+156*_hsv.Saturation-7);Canvas.SetTop(SaturationValueSelector,62+156*(1-_hsv.Value)-7);
            SetInputValidity(true,string.Empty);
        }
        finally{_updating=false;}
    }

    private void SetInputValidity(bool valid,string hint)
    {
        ConfirmButton.IsEnabled=valid;ConfirmButton.Opacity=valid?1:.4;InputHint.Text=hint;
    }

    private void HueMouseDown(object sender,MouseButtonEventArgs e)
    {
        var point=e.GetPosition(HueRing);var distance=(point-new Point(140,140)).Length;
        if(distance<108||distance>140)return;
        HueRing.Focus();HueRing.CaptureMouse();SetHue(ColorSpectrumMath.HueAtPoint(point,new Point(140,140)));e.Handled=true;
    }

    private void HueMouseMove(object sender,MouseEventArgs e)
    {
        if(!HueRing.IsMouseCaptured||e.LeftButton!=MouseButtonState.Pressed)return;
        SetHue(ColorSpectrumMath.HueAtPoint(e.GetPosition(HueRing),new Point(140,140)));e.Handled=true;
    }

    private void PlaneMouseDown(object sender,MouseButtonEventArgs e)
    {
        SaturationValuePlane.Focus();SaturationValuePlane.CaptureMouse();PickPlane(e.GetPosition(SaturationValuePlane));e.Handled=true;
    }

    private void PlaneMouseMove(object sender,MouseEventArgs e)
    {
        if(!SaturationValuePlane.IsMouseCaptured||e.LeftButton!=MouseButtonState.Pressed)return;
        PickPlane(e.GetPosition(SaturationValuePlane));e.Handled=true;
    }

    private void PickPlane(Point point)
    {
        var values=ColorSpectrumMath.ClampSaturationValue(point,SaturationValuePlane.RenderSize);SetSaturationValue(values.X,values.Y);
    }

    private void PaletteMouseUp(object sender,MouseButtonEventArgs e)
    {
        if(sender is not UIElement element||!element.IsMouseCaptured)return;
        if(ReferenceEquals(element,HueRing))SetHue(ColorSpectrumMath.HueAtPoint(e.GetPosition(HueRing),new Point(140,140)));
        else if(ReferenceEquals(element,SaturationValuePlane))PickPlane(e.GetPosition(SaturationValuePlane));
        element.ReleaseMouseCapture();e.Handled=true;
    }

    private void HueKeyDown(object sender,KeyEventArgs e)
    {
        if(e.Key is not (Key.Left or Key.Right or Key.Up or Key.Down))return;
        var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?10:1;
        SetHue(_hsv.Hue+(e.Key is Key.Left or Key.Down?-step:step));e.Handled=true;
    }

    private void PlaneKeyDown(object sender,KeyEventArgs e)
    {
        if(e.Key is not (Key.Left or Key.Right or Key.Up or Key.Down))return;
        var step=Keyboard.Modifiers.HasFlag(ModifierKeys.Shift)?.1:.01;
        SetSaturationValue(_hsv.Saturation+(e.Key==Key.Left?-step:e.Key==Key.Right?step:0),_hsv.Value+(e.Key==Key.Down?-step:e.Key==Key.Up?step:0));e.Handled=true;
    }

    private static BitmapSource CreateHueRing()
    {
        const int size=560;var pixels=new byte[size*size*4];
        for(var y=0;y<size;y++)
        for(var x=0;x<size;x++)
        {
            var dx=(x+.5)/2-140;var dy=(y+.5)/2-140;var radius=Math.Sqrt(dx*dx+dy*dy);
            var coverage=Math.Clamp(Math.Min(134-radius,radius-114)*2,0,1);if(coverage<=0)continue;
            var color=ColorSpectrumMath.ToRgb(new HsvColor(Math.Atan2(dy,dx)*180/Math.PI,1,1));var offset=(y*size+x)*4;
            pixels[offset]=color.B;pixels[offset+1]=color.G;pixels[offset+2]=color.R;pixels[offset+3]=(byte)Math.Round(coverage*255);
        }
        var bitmap=BitmapSource.Create(size,size,192,192,PixelFormats.Bgra32,null,pixels,size*4);bitmap.Freeze();return bitmap;
    }

    private void ConfirmClick(object sender,RoutedEventArgs e){if(ConfirmButton.IsEnabled)DialogResult=true;}
    private void CancelClick(object sender,RoutedEventArgs e)=>Close();
    private void TitleMouseDown(object sender,MouseButtonEventArgs e){if(e.ChangedButton==MouseButton.Left)DragMove();}
}
