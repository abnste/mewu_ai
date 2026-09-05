using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal enum MewuDialogResult{Primary,Secondary,Cancel}

public partial class MewuDialogWindow:Window
{
    private MewuDialogResult _result=MewuDialogResult.Cancel;

    private MewuDialogWindow(string title,string message,string primaryText,string secondaryText,string cancelText)
    {
        InitializeComponent();Title=DialogTitle.Text=LocalizationService.TranslateUiText(title);DialogMessage.Text=LocalizationService.TranslateUiText(message);AddButton(LocalizationService.TranslateUiText(cancelText),"DialogButton",MewuDialogResult.Cancel,false);AddButton(LocalizationService.TranslateUiText(secondaryText),"DialogButton",MewuDialogResult.Secondary,false);AddButton(LocalizationService.TranslateUiText(primaryText),"PrimaryDialogButton",MewuDialogResult.Primary,true);SourceInitialized+=(_,_)=>{var handle=new WindowInteropHelper(this).Handle;NativeMethods.TryUseSystemRoundedCorners(handle);NativeMethods.ApplyPresentationCaptureVisibility(handle,Owner is CaptureOverlayWindow {IsTeachingMode:true});};
    }

    internal static MewuDialogResult ShowChoice(Window owner,string title,string message,string primaryText,string secondaryText,string cancelText="取消")
    {
        var dialog=new MewuDialogWindow(title,message,primaryText,secondaryText,cancelText){Owner=owner,Topmost=owner.Topmost};dialog.ShowDialog();return dialog._result;
    }

    internal static void ShowMessage(Window owner,string title,string message,bool success=false)
    {
        var dialog=new MewuDialogWindow(title,message,LocalizationService.T("确定","OK"),string.Empty,string.Empty){Owner=owner,Topmost=owner.Topmost};
        dialog.DialogSymbol.Text=success?"✓":"!";
        dialog.PreviewKeyDown+=(_,e)=>{if(e.Key==Key.Escape){e.Handled=true;dialog.Close();}};
        dialog.ShowDialog();
    }

    private void AddButton(string text,string style,MewuDialogResult result,bool isDefault)
    {
        if(string.IsNullOrEmpty(text))return;
        var button=new Button{Content=text,Style=(Style)FindResource(style),IsDefault=isDefault};button.Click+=(_,_)=>{_result=result;DialogResult=true;};DialogButtons.Children.Add(button);
    }
    private void CloseClick(object sender,RoutedEventArgs e)=>Close();
    private void TitleMouseDown(object sender,MouseButtonEventArgs e){if(e.ChangedButton==MouseButton.Left)DragMove();}
}
