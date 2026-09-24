// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;
public sealed class SelectionDragPreviewTests
{
    [Theory]
    [InlineData(96)]
    [InlineData(168)]
    public async Task DragPreviewMatchesFinalCropAtDifferentDpi(int dpi)
    {
        var done=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>
        {
            try
            {
                var pixels=new byte[80*60*4];
                for(var y=0;y<60;y++)for(var x=0;x<80;x++){var i=(y*80+x)*4;pixels[i]=(byte)(x*3);pixels[i+1]=(byte)(y*4);pixels[i+3]=255;}
                var source=BitmapSource.Create(80,60,dpi,dpi,PixelFormats.Pbgra32,null,pixels,320);source.Freeze();
                var cache=new SelectionImageCache();var previous=cache.GetPreview(source,new Int32Rect(0,0,1,1));
                foreach(var bounds in new[]{new Int32Rect(12,8,40,30),new Int32Rect(0,0,80,60),new Int32Rect(65,50,15,10)})
                {
                    var brush=cache.GetPreview(source,bounds);Assert.Same(previous,brush);Assert.Same(source,brush.ImageSource);
                    var visual=new DrawingVisual();using(var dc=visual.RenderOpen())dc.DrawRectangle(brush,null,new Rect(0,0,bounds.Width,bounds.Height));
                    var rendered=new RenderTargetBitmap(bounds.Width,bounds.Height,96,96,PixelFormats.Pbgra32);rendered.Render(visual);
                    var actual=new byte[bounds.Width*bounds.Height*4];var expected=new byte[actual.Length];
                    rendered.CopyPixels(actual,bounds.Width*4,0);cache.Get(source,bounds).CopyPixels(expected,bounds.Width*4,0);
                    Assert.Equal(expected,actual);
                }
                cache.Clear();Assert.Null(previous.ImageSource);done.SetResult();
            }
            catch(Exception ex){done.SetException(ex);}
        }){IsBackground=true};thread.SetApartmentState(ApartmentState.STA);thread.Start();await done.Task.WaitAsync(TimeSpan.FromSeconds(15),TestContext.Current.CancellationToken);
    }
}
