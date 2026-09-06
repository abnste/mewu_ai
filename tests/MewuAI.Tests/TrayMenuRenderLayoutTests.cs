using System.Drawing;
using System.Runtime.ExceptionServices;
using mewu_ai_Assistant.Services;
using Xunit;
using Forms=System.Windows.Forms;

namespace MewuAI.Tests;

public sealed class TrayMenuRenderLayoutTests
{
    [Theory]
    [InlineData(1F,"设置")]
    [InlineData(1.25F,"打开主界面")]
    [InlineData(1.5F,"退出")]
    [InlineData(2F,"打开主界面")]
    [InlineData(1F,"Settings")]
    [InlineData(1.25F,"Open MewuAI")]
    [InlineData(1.5F,"Quit")]
    [InlineData(2F,"Open MewuAI")]
    public void CustomHeightMenuTextIsCenteredInEachActualRow(float scale,string label)
    {
        RunSta(()=>
        {
            using var font=new Font(label[0]<128?"Segoe UI":"Microsoft YaHei UI",9F*scale);
            using var menu=new Forms.ContextMenuStrip
            {
                Font=font,ShowCheckMargin=false,ShowImageMargin=false,
                Padding=new Forms.Padding(6),MinimumSize=new Size((int)(196*scale),0),
                Renderer=new AppHost.LightTrayMenuRenderer()
            };
            for(var index=0;index<3;index++)
                menu.Items.Add(new Forms.ToolStripMenuItem(label)
                {
                    AutoSize=false,Size=new Size((int)(184*scale),(int)(36*scale)),
                    Margin=new Forms.Padding(0,1,0,1),Padding=new Forms.Padding(12,0,14,0),
                    TextAlign=ContentAlignment.MiddleLeft
                });
            menu.PerformLayout();
            var rendered=new Dictionary<Forms.ToolStripItem,(Rectangle Bounds,Forms.TextFormatFlags Format)>();
            menu.Renderer.RenderItemText+=(_,args)=>rendered[args.Item]=(args.TextRectangle,args.TextFormat);
            using var bitmap=new Bitmap(menu.Width,menu.Height);
            menu.DrawToBitmap(bitmap,new Rectangle(Point.Empty,bitmap.Size));
            Assert.Equal(3,rendered.Count);
            foreach(var (item,text) in rendered)
            {
                Assert.InRange(Math.Abs(text.Bounds.Top+text.Bounds.Height/2D-item.Height/2D),0,0.5);
                Assert.True(text.Format.HasFlag(Forms.TextFormatFlags.VerticalCenter));
                Assert.True(text.Format.HasFlag(Forms.TextFormatFlags.SingleLine));
                Assert.False(text.Format.HasFlag(Forms.TextFormatFlags.Bottom));
                Assert.True(text.Bounds.Left>=0);
                Assert.True(text.Bounds.Right<=item.Width);
            }
        });
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;
        var thread=new Thread(()=>{try{action();}catch(Exception error){failure=error;}}){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(20)),"Tray rendering did not finish.");
        if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }

    [Fact]
    public void HoverBoundsUseItemLocalCoordinates()
    {
        Assert.Equal(new Rectangle(2,1,152,34),TrayMenuRenderLayout.GetHoverBounds(new Size(156,36)));
    }

    [Theory]
    [InlineData(4,36)]
    [InlineData(156,2)]
    [InlineData(0,0)]
    public void HoverBoundsRejectItemsWithoutDrawableInterior(int width,int height)
    {
        Assert.True(TrayMenuRenderLayout.GetHoverBounds(new Size(width,height)).IsEmpty);
    }
}
