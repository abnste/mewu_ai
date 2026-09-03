using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace mewu_ai_Assistant.Services;

internal static class ScrollingCaptureComposer
{
    internal const long MaxOutputPixels=80_000_000;

    internal static int EstimateVerticalShift(BitmapSource previous,BitmapSource current)
        =>EstimateVerticalShift(previous,current,out _,null);

    internal static int EstimateVerticalShift(BitmapSource previous,BitmapSource current,out double matchScore)
        =>EstimateVerticalShift(previous,current,out matchScore,null);

    internal static int EstimateVerticalShift(BitmapSource previous,BitmapSource current,out double matchScore,Int32Rect? ignoredRegion)
        =>EstimateVerticalShift(previous,current,out matchScore,ignoredRegion,0);

    internal static int EstimateVerticalShift(BitmapSource previous,BitmapSource current,out double matchScore,Int32Rect? ignoredRegion,int preferredDirection)
    {
        matchScore=double.PositiveInfinity;
        if(previous.PixelWidth!=current.PixelWidth||previous.PixelHeight!=current.PixelHeight)return 0;
        var width=previous.PixelWidth;var height=previous.PixelHeight;
        if(width<8||height<32)return 0;

        // Compare compact luminance/edge grids rather than whole-frame means.
        // Blank backgrounds contribute no weight, while text and image detail
        // keep their position signal. The unshifted frame is scored too: a
        // cursor animation or other local change must not be mistaken for page
        // movement unless a translated overlap is materially better.
        var first=FeatureGrid.Create(previous,ignoredRegion);var second=FeatureGrid.Create(current,ignoredRegion);
        var stationary=Difference(first,second,0);
        if(stationary<=1.5){matchScore=stationary;return 0;}

        var maximum=Math.Min(height-16,(int)(height*.84));var bestShift=0;var bestScore=double.PositiveInfinity;
        for(var shift=1;shift<=maximum;shift++)
        {
            if(preferredDirection>=0)
            {
                var downwardScore=Difference(first,second,shift);
                if(IsBetterMatch(downwardScore,shift,bestScore,bestShift)){bestScore=downwardScore;bestShift=shift;}
            }
            if(preferredDirection<=0)
            {
                var upwardScore=Difference(second,first,shift);
                if(IsBetterMatch(upwardScore,-shift,bestScore,bestShift)){bestScore=upwardScore;bestShift=-shift;}
            }
        }
        matchScore=bestScore;
        if(bestShift==0||double.IsPositiveInfinity(bestScore)||bestScore>34)return 0;
        return preferredDirection!=0||stationary>bestScore*.9?bestShift:0;
    }

    private static bool IsBetterMatch(double candidateScore,int candidateShift,double bestScore,int bestShift)
    {
        const double tieTolerance=.08;
        return candidateScore<bestScore-tieTolerance||Math.Abs(candidateScore-bestScore)<=tieTolerance&&Math.Abs(candidateShift)<Math.Abs(bestShift);
    }

    private static double Difference(FeatureGrid first,FeatureGrid second,int shift)
    {
        var overlap=first.Rows-shift;if(overlap<=0)return double.PositiveInfinity;
        var edgeRows=Math.Clamp(first.Rows/160,4,10);var edgeColumns=Math.Clamp(first.Columns/120,1,3);var stepY=Math.Max(1,first.Rows/300);var start=Math.Max(edgeRows,Math.Min(overlap/5,Math.Max(0,first.Rows/20)));var end=overlap-edgeRows;double difference=0;var informative=0;
        for(var y=start;y<end;y+=stepY)
        {
            var firstRow=(y+shift)*first.Columns;var secondRow=y*second.Columns;
            for(var column=edgeColumns;column<first.Columns-edgeColumns;column++)
            {
                var firstIndex=firstRow+column;var secondIndex=secondRow+column;var activity=Math.Max(first.Activity[firstIndex],second.Activity[secondIndex]);
                if(first.Ignored[firstIndex]||second.Ignored[secondIndex])continue;
                if(activity<5)continue;
                difference+=Math.Abs(first.Mean[firstIndex]-second.Mean[secondIndex])+Math.Abs(first.Activity[firstIndex]-second.Activity[secondIndex])*.25;
                informative++;
            }
        }
        var sampledRows=Math.Max(1,(Math.Max(start,end)-start+stepY-1)/stepY);var minimum=Math.Max(24,sampledRows*2);
        return informative<minimum?double.PositiveInfinity:difference/informative;
    }

    private sealed class FeatureGrid
    {
        private FeatureGrid(int columns,int rows,byte[] mean,byte[] activity,bool[] ignored){Columns=columns;Rows=rows;Mean=mean;Activity=activity;Ignored=ignored;}
        internal int Columns{get;}
        internal int Rows{get;}
        internal byte[] Mean{get;}
        internal byte[] Activity{get;}
        internal bool[] Ignored{get;}

        internal static FeatureGrid Create(BitmapSource source,Int32Rect? ignoredRegion)
        {
            var pixels=Pixels(source);var width=source.PixelWidth;var height=source.PixelHeight;var stride=width*4;var columns=Math.Clamp(width/6,64,256);
            var mean=new byte[checked(columns*height)];var range=new byte[mean.Length];var activity=new byte[mean.Length];var ignoredCells=new bool[mean.Length];
            for(var y=0;y<height;y++)for(var column=0;column<columns;column++)
            {
                var fromX=column*width/columns;var toX=Math.Max(fromX+1,(column+1)*width/columns);var minimum=255;var maximum=0;var total=0;var count=0;
                for(var x=fromX;x<toX;x++)
                {
                    var pixel=y*stride+x*4;var luminance=(pixels[pixel]*29+pixels[pixel+1]*150+pixels[pixel+2]*77)>>8;
                    minimum=Math.Min(minimum,luminance);maximum=Math.Max(maximum,luminance);total+=luminance;count++;
                }
                var index=y*columns+column;mean[index]=(byte)(total/Math.Max(1,count));range[index]=(byte)(maximum-minimum);
            }
            for(var y=0;y<height;y++)for(var column=0;column<columns;column++)
            {
                var index=y*columns+column;var fromX=column*width/columns;var toX=Math.Max(fromX+1,(column+1)*width/columns);
                var ignored=ignoredRegion is { } region&&y>=region.Y&&y<region.Y+region.Height&&fromX<region.X+region.Width&&toX>region.X;
                if(ignored){ignoredCells[index]=true;activity[index]=0;continue;}
                var value=(int)range[index];if(column>0)value=Math.Max(value,Math.Abs(mean[index]-mean[index-1]));if(y>0)value=Math.Max(value,Math.Abs(mean[index]-mean[index-columns]));activity[index]=(byte)Math.Min(255,value);
            }
            return new FeatureGrid(columns,height,mean,activity,ignoredCells);
        }
    }

    internal static BitmapSource Compose(IReadOnlyList<BitmapSource> frames)
        =>Compose(frames,null);

    internal static BitmapSource Compose(IReadOnlyList<BitmapSource> frames,IReadOnlyList<int>? knownShifts)
    {
        ArgumentNullException.ThrowIfNull(frames);
        if(frames.Count==0)throw new ArgumentException("长截图至少需要一帧",nameof(frames));
        if(knownShifts is not null&&knownShifts.Count!=frames.Count-1)throw new ArgumentException("长截图位移数量不匹配",nameof(knownShifts));
        var width=frames[0].PixelWidth;var height=frames[0].PixelHeight;var shifts=new List<int>();var origins=new List<int>{0};var minimumOrigin=0;var maximumBottom=height;
        for(var index=1;index<frames.Count;index++)
        {
            if(frames[index].PixelWidth!=width||frames[index].PixelHeight!=height)throw new ArgumentException("长截图帧尺寸不一致",nameof(frames));
            var shift=knownShifts is null?EstimateVerticalShift(frames[index-1],frames[index]):knownShifts[index-1];
            if(shift==0||Math.Abs(shift)>=height)break;
            var origin=checked(origins[^1]+shift);var nextMinimum=Math.Min(minimumOrigin,origin);var nextMaximum=Math.Max(maximumBottom,checked(origin+height));
            if(checked((long)(nextMaximum-nextMinimum)*width)>MaxOutputPixels)break;
            shifts.Add(shift);origins.Add(origin);minimumOrigin=nextMinimum;maximumBottom=nextMaximum;
        }
        var totalHeight=checked(maximumBottom-minimumOrigin);var stride=checked(width*4);var output=new byte[checked(stride*totalHeight)];var first=Pixels(frames[0]);Buffer.BlockCopy(first,0,output,checked(-minimumOrigin*stride),first.Length);var coveredTop=0;var coveredBottom=height;
        for(var index=0;index<shifts.Count;index++)
        {
            var origin=origins[index+1];var bottom=origin+height;var pixels=Pixels(frames[index+1]);
            if(origin<coveredTop)
            {
                var novelRows=coveredTop-origin;Buffer.BlockCopy(pixels,0,output,checked((origin-minimumOrigin)*stride),checked(novelRows*stride));coveredTop=origin;
            }
            if(bottom>coveredBottom)
            {
                var sourceRow=coveredBottom-origin;var novelRows=bottom-coveredBottom;Buffer.BlockCopy(pixels,checked(sourceRow*stride),output,checked((coveredBottom-minimumOrigin)*stride),checked(novelRows*stride));coveredBottom=bottom;
            }
        }
        var result=BitmapSource.Create(width,totalHeight,frames[0].DpiX,frames[0].DpiY,PixelFormats.Bgra32,null,output,stride);result.Freeze();return result;
    }

    private static byte[] Pixels(BitmapSource source)
    {
        var formatted=source.Format==PixelFormats.Bgra32?source:new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);var stride=formatted.PixelWidth*4;var result=new byte[stride*formatted.PixelHeight];formatted.CopyPixels(result,stride,0);return result;
    }
}
