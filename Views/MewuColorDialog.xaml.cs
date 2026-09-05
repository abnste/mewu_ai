using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

public partial class MewuColorDialog:Window
{
    private bool _updating;
    internal Color SelectedColor { get; private set; }

    private MewuColorDialog(Color initial)
    {
        InitializeComponent();SelectedColor=initial;SourceInitialized+=(_,_)=>{var handle=new WindowInteropHelper(this).Handle;NativeMethods.TryUseSystemRoundedCorners(handle);NativeMethods.ApplyPresentationCaptureVisibility(handle,Owner is CaptureOverlayWindow {IsTeachingMode:true});};Loaded+=(_,_)=>SetChannels(initial.R,initial.G,initial.B);
    }

    internal static bool TryChoose(Window owner,Color initial,out Color selected)
    {
        var dialog=new MewuColorDialog(initial){Owner=owner,Topmost=owner.Topmost};var accepted=dialog.ShowDialog()==true;selected=accepted?dialog.SelectedColor:initial;return accepted;
    }

    private void SliderChanged(object sender,RoutedPropertyChangedEventArgs<double> e){if(!_updating&&IsLoaded)SetChannels((byte)Math.Round(RedSlider.Value),(byte)Math.Round(GreenSlider.Value),(byte)Math.Round(BlueSlider.Value));}
    private void ValueTextChanged(object sender,System.Windows.Controls.TextChangedEventArgs e)
    {
        if(_updating||!IsLoaded)return;if(byte.TryParse(RedValue.Text,out var red)&&byte.TryParse(GreenValue.Text,out var green)&&byte.TryParse(BlueValue.Text,out var blue))SetChannels(red,green,blue);
    }
    private void SetChannels(byte red,byte green,byte blue)
    {
        _updating=true;try{RedSlider.Value=red;GreenSlider.Value=green;BlueSlider.Value=blue;RedValue.Text=red.ToString();GreenValue.Text=green.ToString();BlueValue.Text=blue.ToString();SelectedColor=Color.FromRgb(red,green,blue);ColorPreview.Background=new SolidColorBrush(SelectedColor);HexPreview.Text=$"#{red:X2}{green:X2}{blue:X2}";HexPreview.Foreground=ContrastBrush(SelectedColor);}finally{_updating=false;}
    }
    private static Brush ContrastBrush(Color color)=>color.R*.299+color.G*.587+color.B*.114>160?Brushes.Black:Brushes.White;
    private void ConfirmClick(object sender,RoutedEventArgs e)=>DialogResult=true;
    private void CancelClick(object sender,RoutedEventArgs e)=>Close();
    private void TitleMouseDown(object sender,MouseButtonEventArgs e){if(e.ChangedButton==MouseButton.Left)DragMove();}
}
