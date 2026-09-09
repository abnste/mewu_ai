// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Shell;
using mewu_ai_Assistant.Interop;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class LicenseNoticesWindow : Window
{
    private sealed record DocumentEntry(string Title,string Path,bool Markdown=false);
    private readonly ComboBox _documents=new(){MinWidth=0,HorizontalAlignment=HorizontalAlignment.Stretch};
    private readonly TextBox _text=new(){IsReadOnly=true,IsUndoEnabled=false,TextWrapping=TextWrapping.Wrap,
        VerticalScrollBarVisibility=ScrollBarVisibility.Auto,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,
        BorderThickness=new Thickness(0),Padding=new Thickness(14),FontSize=13};
    private readonly MarkdownAnswerView _overview=new(){FontSize=13,Padding=new Thickness(14),VerticalScrollBarVisibility=ScrollBarVisibility.Auto};

    internal LicenseNoticesWindow()
    {
        Title=LocalizationService.T("开源许可与第三方声明","Open-source licenses and third-party notices");
        Width=900;Height=640;MinWidth=600;MinHeight=400;WindowStartupLocation=WindowStartupLocation.CenterOwner;
        ShowInTaskbar=false;WindowStyle=WindowStyle.None;Background=new SolidColorBrush(Color.FromRgb(245,247,252));
        UseLayoutRounding=true;SnapsToDevicePixels=true;
        WindowChrome.SetWindowChrome(this,new WindowChrome{CaptionHeight=0,ResizeBorderThickness=new Thickness(6),GlassFrameThickness=new Thickness(0),UseAeroCaptionButtons=false});
        var root=new Grid{Margin=new Thickness(18)};
        root.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});root.RowDefinitions.Add(new RowDefinition{Height=GridLength.Auto});
        root.RowDefinitions.Add(new RowDefinition{Height=new GridLength(1,GridUnitType.Star)});
        var header=new DockPanel{Margin=new Thickness(0,0,0,14),LastChildFill=true};
        var close=new Button{Content="×",Width=32,Height=32,Padding=new Thickness(0),ToolTip=LocalizationService.T("关闭","Close")};
        close.Click+=(_,_)=>Close();DockPanel.SetDock(close,Dock.Right);header.Children.Add(close);
        var title=new TextBlock{Text=Title,FontSize=18,FontWeight=FontWeights.SemiBold,TextWrapping=TextWrapping.Wrap,VerticalAlignment=VerticalAlignment.Center};
        title.MouseLeftButtonDown+=(_,e)=>{if(e.ButtonState==MouseButtonState.Pressed)DragMove();};header.Children.Add(title);root.Children.Add(header);
        var intro=new StackPanel{Margin=new Thickness(0,0,0,12)};
        intro.Children.Add(new TextBlock{Text=LocalizationService.T("感谢以下项目的贡献。第三方组件、模型和资源保留各自版权与许可；完整原文可在此离线查阅。","Thanks to the projects that make MewuAI possible. Third-party components, models, and resources retain their own copyrights and licenses. Full original notices are available offline."),TextWrapping=TextWrapping.Wrap,Foreground=new SolidColorBrush(Color.FromRgb(99,112,137)),Margin=new Thickness(0,0,0,10)});
        _documents.DisplayMemberPath=nameof(DocumentEntry.Title);intro.Children.Add(_documents);Grid.SetRow(intro,1);root.Children.Add(intro);
        var documents=new Grid();documents.Children.Add(_text);documents.Children.Add(_overview);
        LocalizationService.SetExcludeFromLocalization(documents,true);
        var surface=new Border{Background=Brushes.White,BorderBrush=new SolidColorBrush(Color.FromRgb(215,225,239)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(10),Padding=new Thickness(2),Child=documents};
        Grid.SetRow(surface,2);root.Children.Add(surface);Content=root;
        _text.ContextMenu=TextSelectionMenu.Create(()=>_text.SelectedText,()=>_text.Text,text=>ClipboardService.TrySetText(text,out string? _));
        _documents.SelectionChanged+=(_,_)=>ShowDocument();
        SourceInitialized+=(_,_)=>{var handle=new WindowInteropHelper(this).Handle;NativeMethods.TryUseSystemRoundedCorners(handle);NativeMethods.ExcludeFromCapture(handle);};
        PreviewKeyDown+=(_,e)=>{if(e.Key==Key.Escape){Close();e.Handled=true;}};
        LoadDocuments();
    }

    private void LoadDocuments()
    {
        var directory=AppContext.BaseDirectory;
        _documents.Items.Add(new DocumentEntry(LocalizationService.T("第三方组件、版本与来源","Third-party components, versions, and sources"),Path.Combine(directory,"THIRD-PARTY-NOTICES.md"),true));
        _documents.Items.Add(new DocumentEntry(LocalizationService.T("喵呜AI · MPL-2.0 完整许可","MewuAI · Full MPL-2.0 license"),Path.Combine(directory,"LICENSE")));
        _documents.Items.Add(new DocumentEntry(LocalizationService.T("源代码获取与分发说明","Source code and distribution information"),Path.Combine(directory,"SOURCE.md"),true));
        try
        {
            var licenses=Path.Combine(directory,"Licenses");
            if(Directory.Exists(licenses))
                foreach(var file in Directory.EnumerateFiles(licenses,"*.txt").Order(StringComparer.OrdinalIgnoreCase).Take(64))
                    _documents.Items.Add(new DocumentEntry(Path.GetFileNameWithoutExtension(file),file));
        }
        catch(IOException){}catch(UnauthorizedAccessException){}
        _documents.SelectedIndex=0;
    }

    private void ShowDocument()
    {
        if(_documents.SelectedItem is not DocumentEntry document)return;
        _overview.Visibility=Visibility.Collapsed;_text.Visibility=Visibility.Visible;
        try
        {
            using var stream=File.OpenRead(document.Path);
            if(stream.Length>2*1024*1024)throw new IOException("License document exceeds the display limit.");
            using var reader=new StreamReader(stream);var content=reader.ReadToEnd();
            if(document.Markdown){_overview.Markdown=content;_overview.ScrollToHome();_overview.Visibility=Visibility.Visible;_text.Visibility=Visibility.Collapsed;}
            else{_text.Text=content;_text.ScrollToHome();}
        }
        catch(Exception ex)when(ex is IOException or UnauthorizedAccessException)
        {
            _text.Text=LocalizationService.T("无法读取随附的许可文件。请重新安装完整的喵呜AI 安装包。","The bundled license file could not be read. Please reinstall the complete MewuAI package.");
        }
    }
}
