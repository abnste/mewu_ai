using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

[Collection("WPF theme resources")]
public sealed class PinnedVisualSnapshotTests
{
    [Theory]
    [InlineData(1)]
    [InlineData(1.5)]
    [InlineData(2)]
    public void CompletePinSnapshotIncludesTransparentMarginShadowAndContent(double dpiScale)
    {
        Exception? failure=null;
        var thread=new Thread(()=>
        {
            try
            {
                var root=new Grid{Width=120,Height=100,Background=Brushes.Transparent};
                root.Children.Add(new Border{Margin=new Thickness(12),Background=Brushes.White,CornerRadius=new CornerRadius(10),BorderBrush=Brushes.Gray,BorderThickness=new Thickness(1),Effect=new DropShadowEffect{Color=Color.FromRgb(42,55,72),BlurRadius=22,ShadowDepth=4,Opacity=.3}});
                root.Measure(new Size(120,100));root.Arrange(new Rect(0,0,120,100));root.UpdateLayout();
                var width=(int)(120*dpiScale);var height=(int)(100*dpiScale);
                var image=PinnedVisualSnapshotRenderer.Render(root,width,height);
                Assert.True(image.IsFrozen);Assert.Equal(width,image.PixelWidth);Assert.Equal(height,image.PixelHeight);
                var pixels=new byte[width*height*4];image.CopyPixels(pixels,width*4,0);
                Assert.Equal(255,pixels[(height/2*width+width/2)*4+3]);
                var shadowFound=false;
                for(var y=(int)(25*dpiScale);y<(int)(70*dpiScale);y++)
                    for(var x=(int)(3*dpiScale);x<(int)(11*dpiScale);x++)
                        if(pixels[(y*width+x)*4+3] is >0 and <255)shadowFound=true;
                Assert.True(shadowFound,"Snapshot omitted the shadow outside the content rectangle.");
                Assert.True(pixels[3]<40,"Outer corner must remain transparent rather than becoming an opaque rectangle.");
                using var desktop=new System.Drawing.Bitmap(width,height);
                using(var graphics=System.Drawing.Graphics.FromImage(desktop))graphics.Clear(System.Drawing.Color.White);
                var bounds=new mewu_ai_Assistant.Models.ScreenRect(-30,-20,width,height);
                using var registration=PinnedImageCaptureRegistry.Register(()=>new PinnedImageCaptureSnapshot(bounds,image,.8));
                PinnedImageCaptureRegistry.CompositeInto(desktop,bounds);
                Assert.Equal(255,desktop.GetPixel(width/2,height/2).R);
                Assert.True(desktop.GetPixel((int)(10*dpiScale),height/2).R<255,"Recapture compositing lost the shadow.");
            }
            catch(Exception ex){failure=ex;}
        }){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();Assert.True(thread.Join(TimeSpan.FromSeconds(15)));
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
