// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Net.Http;
using mewu_ai_Assistant.Services;

namespace mewu_ai_Assistant.Views;

internal sealed class ReplyImageView : Border
{
    private readonly string _source;
    private readonly Image _image=new(){Stretch=Stretch.Uniform,MaxHeight=300,HorizontalAlignment=HorizontalAlignment.Left};
    private readonly TextBlock _status=new(){TextWrapping=TextWrapping.Wrap,Margin=new Thickness(8),Foreground=Brushes.SlateGray};
    private CancellationTokenSource? _loading;
    private bool _finished;
    private MarkdownAnswerView? _owner;
    internal ReplyImageView(string source,string description)
    {
        _source=source;Description=description;Width=400;MaxWidth=400;MinHeight=32;Margin=new Thickness(0,5,0,5);
        CornerRadius=new CornerRadius(8);Background=new SolidColorBrush(Color.FromRgb(245,247,251));
        var content=new Grid();content.Children.Add(_image);content.Children.Add(_status);Child=content;
        _status.Text=LocalizationService.T("图片加载中…","Loading image…");
        ToolTip=description;Loaded+=OnLoaded;Unloaded+=(_,_)=>{_loading?.Cancel();_loading=null;_owner?.ReleaseReplyImage(this);if(_owner is not null)_owner.SizeChanged-=OwnerSizeChanged;_owner=null;};
    }
    internal string Description {get;}
    private async void OnLoaded(object sender,RoutedEventArgs e)
    {
        _owner=FindAnswer();
        if(_owner is not null)
        {
            if(!_owner.RegisterReplyImage(this)){_status.Text=LocalizationService.T("此回复最多显示 16 张图片","Up to 16 images can be displayed in one reply");return;}
            _owner.SizeChanged-=OwnerSizeChanged;_owner.SizeChanged+=OwnerSizeChanged;OwnerSizeChanged(null,null!);
        }
        if(_finished||_loading is not null)return;
        var operation=new CancellationTokenSource();_loading=operation;
        try
        {
            var local=ReplyImageService.TryGetLocalPath(_source,out _);
            if(local&&_owner?.CanLoadLocalReplyImage(_source)!=true)
            {
                _status.Text=LocalizationService.T("此回复未提供可读取的本地图片","This reply did not provide an accessible local image");_finished=true;return;
            }
            var image=await ReplyImageService.LoadAsync(_source,operation.Token,local);
            if(!ReferenceEquals(_loading,operation)||operation.IsCancellationRequested||!IsLoaded)return;
            _image.Source=image;_status.Visibility=Visibility.Collapsed;_finished=true;
        }
        catch(OperationCanceledException){if(ReferenceEquals(_loading,operation)&&IsLoaded){_status.Text=LocalizationService.T("图片加载超时","Image loading timed out");_finished=true;}}
        catch(Exception ex)when(ex is System.IO.FileNotFoundException or System.IO.DirectoryNotFoundException){if(ReferenceEquals(_loading,operation)&&IsLoaded){_status.Text=LocalizationService.T("Hermes 返回的图片文件已不存在，请让它重新生成或发送图片","The image file returned by Hermes no longer exists. Ask Hermes to generate or send it again.");_finished=true;}}
        catch(Exception ex)when(ex is System.IO.IOException or UnauthorizedAccessException or System.Security.SecurityException or HttpRequestException or FormatException or NotSupportedException or ArgumentException or InvalidOperationException or System.Net.WebException or System.Runtime.InteropServices.COMException)
        {
            if(ReferenceEquals(_loading,operation)&&IsLoaded){_status.Text=LocalizationService.T("图片暂时无法显示","Image unavailable");_finished=true;}
        }
        finally{if(ReferenceEquals(_loading,operation))_loading=null;operation.Dispose();}
    }
    private void OwnerSizeChanged(object? sender,SizeChangedEventArgs e){if(_owner is not null)Width=Math.Max(1,Math.Min(400,_owner.ActualWidth-24));}
    private MarkdownAnswerView? FindAnswer()
    {
        for(DependencyObject? node=this;node is not null;node=VisualTreeHelper.GetParent(node))if(node is MarkdownAnswerView answer)return answer;
        return null;
    }
}
