// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class TranslationOverlayLayoutServiceTests
{
    [Theory]
    [InlineData(.5)]
    [InlineData(1)]
    [InlineData(1.75)]
    public void LongRightHandLabelKeepsItsOriginalTextOrigin(double scale)
    {
        var source=new Rect(430*scale,40*scale,60*scale,20*scale);
        var cell=TranslationOverlayLayoutService.AnchorCell(source,new Rect(0,0,500*scale,240*scale));
        var placement=TranslationOverlayLayoutService.PlaceWithin(source,cell,220*scale,75*scale);
        Assert.Equal(source.Left,placement.Left+TranslationOverlayLayoutService.HorizontalPadding/2,8);
        Assert.Equal(source.Top,placement.Top+TranslationOverlayLayoutService.VerticalPadding/2,8);
        Assert.True(placement.Contains(source));
        Assert.True(placement.Right<=500*scale);
    }

    [Fact]
    public void BottomAndRightEdgeTextUsesRemainingSpaceInsteadOfMovingToAnotherLine()
    {
        var source=new Rect(470,275,30,25);
        var placement=TranslationOverlayLayoutService.PlaceWithin(source,new Rect(0,0,500,300),350,100);
        Assert.Equal(467,placement.Left);Assert.Equal(274,placement.Top);
        Assert.Equal(500,placement.Right);Assert.Equal(300,placement.Bottom);
    }

    [Fact]
    public void DenseTranslatedRowsCannotOverlapEvenWhenTheyGrow()
    {
        var lines=Enumerable.Range(0,12).Select(i=>new Rect(20,10+i*19,130,16)).ToArray();
        var cells=TranslationOverlayLayoutService.AllocateCells(lines,new Size(400,260));
        var placements=lines.Select((line,i)=>TranslationOverlayLayoutService.PlaceWithin(line,cells[i],360,60)).ToArray();
        AssertDisjoint(placements,new Rect(0,0,400,260));
        Assert.All(Enumerable.Range(0,lines.Length),i=>Assert.True(cells[i].Contains(lines[i])));
    }

    [Fact]
    public void ColumnsStaySeparateAndUnsortedOcrKeepsItsIdentity()
    {
        Rect[] lines=[new(310,60,160,18),new(20,20,150,18),new(310,20,160,18),new(20,60,150,18)];
        var cells=TranslationOverlayLayoutService.AllocateCells(lines,new Size(520,120));
        AssertDisjoint(cells,new Rect(0,0,520,120));
        Assert.All(Enumerable.Range(0,lines.Length),i=>Assert.True(cells[i].Contains(lines[i])));
        Assert.True(cells[1].Right<=cells[2].Left);Assert.True(cells[2].Bottom<=cells[0].Top);
    }

    [Fact]
    public void DuplicateAndOverlappingOcrTerminatesWithDisjointCells()
    {
        var lines=Enumerable.Repeat(new Rect(10,10,80,20),128).ToArray();
        AssertDisjoint(TranslationOverlayLayoutService.AllocateCells(lines,new Size(400,300)),new Rect(0,0,400,300));
    }

    private static void AssertDisjoint(IReadOnlyList<Rect> rectangles,Rect bounds)
    {
        for(var i=0;i<rectangles.Count;i++)
        {
            Assert.False(rectangles[i].IsEmpty);Assert.True(bounds.Contains(rectangles[i]));
            for(var j=0;j<i;j++){var overlap=Rect.Intersect(rectangles[i],rectangles[j]);Assert.True(overlap.IsEmpty||overlap.Width*overlap.Height<.00001);}
        }
    }

    [Fact]
    public void TranslationExpandsBeyondTheOriginalOcrLineInsteadOfClippingText()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(20,30,60,20),new Size(500,300),240,24);
        Assert.Equal(new Rect(20,28,240,24),result);
    }

    [Fact]
    public void ExpandedTranslationShiftsLeftAtTheRightEdge()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(430,30,60,20),new Size(500,300),220,24);
        Assert.Equal(new Rect(280,28,220,24),result);
    }

    [Fact]
    public void OversizedTranslationStaysInsideTheSelectionWithoutBeingTruncatedVertically()
    {
        var result=TranslationOverlayLayoutService.Place(new Rect(10,260,80,20),new Size(320,300),600,80);
        Assert.Equal(new Rect(0,220,320,80),result);
    }

    [Fact]
    public void LongTranslationWrapsOnlyAtTheSelectionEdgeWithoutDroppingCharacters()
    {
        const string text="一段非常非常长的译文内容";var rows=TranslationOverlayLayoutService.WrapText(text,50,value=>value.Length*10);
        Assert.True(rows.Count>1);Assert.Equal(text,string.Concat(rows));Assert.All(rows,row=>Assert.True(row.Length<=5));
    }

    [Fact]
    public void TranslationBackdropUsesGaussianSourcePixelsWithoutATextBoxBorder()
    {
        RunSta(() =>
        {
            var source=Checkerboard(160,60);var bounds=new Rect(20,10,120,36);var region=TranslationOverlayLayoutService.ToImagePixelRect(bounds,source,1,1);var average=TranslationOverlayLayoutService.GetAverageColor(source,region);var backdrop=TranslationOverlayLayoutService.CreateBackdrop(source,bounds,1,1,average);var canvas=Assert.IsType<Canvas>(Assert.Single(backdrop.Children));var image=Assert.IsType<Image>(canvas.Children[0]);var blur=Assert.IsType<BlurEffect>(image.Effect);Assert.Equal(KernelType.Gaussian,blur.KernelType);Assert.True(blur.Radius>=8);Assert.DoesNotContain(canvas.Children.Cast<UIElement>(),element=>element is Border);
            backdrop.Measure(new Size(bounds.Width,bounds.Height));backdrop.Arrange(new Rect(0,0,bounds.Width,bounds.Height));var rendered=new RenderTargetBitmap((int)bounds.Width,(int)bounds.Height,96,96,PixelFormats.Pbgra32);rendered.Render(backdrop);var pixel=new byte[4];rendered.CopyPixels(new Int32Rect((int)bounds.Width/2,(int)bounds.Height/2,1,1),pixel,4,0);Assert.True(pixel[3]>0);Assert.InRange(pixel[0],25,230);Assert.InRange(pixel[1],25,230);Assert.InRange(pixel[2],25,230);
        });
    }

    [Fact]
    public void InvalidOcrBoundsDoNotAffectValidTranslationCells()
    {
        var cells=TranslationOverlayLayoutService.AllocateCells([Rect.Empty,new Rect(double.NaN,0,10,10),new Rect(0,0,double.PositiveInfinity,10),new Rect(10,10,40,20)],new Size(200,100));
        Assert.All(cells.Take(3),cell=>Assert.True(cell.IsEmpty));
        Assert.Equal(new Rect(0,0,200,100),cells[3]);
    }

    private static BitmapSource Checkerboard(int width,int height)
    {
        var stride=width*4;var pixels=new byte[stride*height];for(var y=0;y<height;y++)for(var x=0;x<width;x++){var value=(byte)(((x/4+y/4)&1)==0?20:235);var offset=y*stride+x*4;pixels[offset]=pixels[offset+1]=pixels[offset+2]=value;pixels[offset+3]=255;}var bitmap=BitmapSource.Create(width,height,96,96,PixelFormats.Bgra32,null,pixels,stride);bitmap.Freeze();return bitmap;
    }

    private static void RunSta(Action action)
    {
        Exception? failure=null;var thread=new Thread(()=>{try{action();}catch(Exception ex){failure=ex;}});thread.SetApartmentState(ApartmentState.STA);thread.Start();thread.Join();if(failure is not null)ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
