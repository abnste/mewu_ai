using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;

namespace mewu_ai_Assistant.Views;

internal enum MewuDialogResult{Primary,Secondary,Cancel}

public partial class MewuDialogWindow:Window
{
    private MewuDialogResult _result=MewuDialogResult.Cancel;

    private MewuDialogWindow(string title,string message,string primaryText,string secondaryText,string cancelText)
    {
        InitializeComponent();Title=DialogTitle.Text=title;DialogMessage.Text=message;AddButton(cancelText,"DialogButton",MewuDialogResult.Cancel,false);AddButton(secondaryText,"DialogButton",MewuDialogResult.Secondary,false);AddButton(primaryText,"PrimaryDialogButton",MewuDialogResult.Primary,true);SourceInitialized+=(_,_)=>{var handle=new WindowInteropHelper(this).Handle;NativeMethods.TryUseSystemRoundedCorners(handle);NativeMethods.ExcludeFromCapture(handle);};
    }

    internal static MewuDialogResult ShowChoice(Window owner,string title,string message,string primaryText,string secondaryText,string cancelText="取消")
    {
        var dialog=new MewuDialogWindow(title,message,primaryText,secondaryText,cancelText){Owner=owner,Topmost=owner.Topmost};dialog.ShowDialog();return dialog._result;
    }

    private void AddButton(string text,string style,MewuDialogResult result,bool isDefault)
    {
        var button=new Button{Content=text,Style=(Style)FindResource(style),IsDefault=isDefault};button.Click+=(_,_)=>{_result=result;DialogResult=true;};DialogButtons.Children.Add(button);
    }
    private void CloseClick(object sender,RoutedEventArgs e)=>Close();
    private void TitleMouseDown(object sender,MouseButtonEventArgs e){if(e.ChangedButton==MouseButton.Left)DragMove();}
}
