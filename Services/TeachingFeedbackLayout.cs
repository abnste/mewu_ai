// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class TeachingFeedbackLayout
{
    internal static DrawingImage DrawText(string value,double width,double size,Brush brush,bool halo=false)
    {
        var group=new DrawingGroup();var y=0d;
        using(var dc=group.Open())
        {
            foreach(var line in value.Replace("\r","").Split('\n').Take(48))
            {
                if(MathFormulaRenderer.Create(line,20,brush,halo:halo) is { } math)
                {
                    var scale=Math.Min(size/20,width/math.Width);var height=math.Height*scale;
                    dc.DrawImage(math,new Rect(0,y,math.Width*scale,height));y+=height+size*.3;
                }
                else
                {
                    var text=new FormattedText(line.Length==0?" ":line,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,new Typeface("Segoe UI, Microsoft YaHei UI"),size,brush,1){MaxTextWidth=Math.Max(1,width)};
                    if(halo)dc.DrawGeometry(Brushes.White,new Pen(Brushes.White,size*.14),text.BuildGeometry(new Point(0,y)));
                    dc.DrawText(text,new Point(0,y));y+=text.Height+size*.2;
                }
            }
            dc.DrawRectangle(Brushes.Transparent,null,new Rect(0,0,width,Math.Max(1,y)));
        }
        group.Freeze();var image=new DrawingImage(group);image.Freeze();return image;
    }
    internal static IReadOnlyList<AiAnnotation> Annotations(TeachingPage page,string handle)
    {
        var output=new List<AiAnnotation>();var occupied=page.Items.Select(i=>new Rect(i.X,i.Y,i.Width,i.Height)).ToList();
        var ink=new InkMap(page.Image);
        foreach(var item in page.Items.Take(24))
        {
            var color=item.Verdict==GradingVerdict.Correct?"#20966C":item.Verdict==GradingVerdict.Uncertain?"#B77713":"#D74655";
            var marker=item.Verdict switch{GradingVerdict.Correct=>"✓",GradingVerdict.Incorrect=>"×",GradingVerdict.Blank=>"—",_=>"?"};
            var title=$"{item.Question}  {marker}  {TeachingSession.VerdictText(item.Verdict)}"+(item.Confirmed?"":LocalizationService.T("（初稿）"," (draft)"));
            var text=title+(item.Verdict==GradingVerdict.Incorrect||item.Verdict==GradingVerdict.Blank?"\n"+item.Expected:"")+(item.Reason.Length>0?"\n"+item.Reason:"");
            const double font=.012;const double width=.32;
            var drawing=DrawText(text,width*page.Image.PixelWidth,font*page.Image.PixelHeight,Brushes.Black);
            var height=drawing.Height/page.Image.PixelHeight;
            var target=new Rect(item.X,item.Y,item.Width,item.Height);
            Rect Pixels(Rect r)=>new(r.X*page.Image.PixelWidth,r.Y*page.Image.PixelHeight,r.Width*page.Image.PixelWidth,r.Height*page.Image.PixelHeight);
            var plan=AnnotationLayoutService.FindCalloutPlacement(Pixels(target),new Size(width*page.Image.PixelWidth,height*page.Image.PixelHeight),new Size(page.Image.PixelWidth,page.Image.PixelHeight),occupied.Select(Pixels).ToArray(),.018*page.Image.PixelWidth,.025*page.Image.PixelWidth);
            var placed=plan.CardBounds;var bounds=new Rect(placed.X/page.Image.PixelWidth,placed.Y/page.Image.PixelHeight,placed.Width/page.Image.PixelWidth,placed.Height/page.Image.PixelHeight);
            var candidates=new List<Rect>{bounds};
            foreach(var x in new[]{target.Right+.018,target.Left-width-.018})
                foreach(var y in new[]{target.Top,target.Top+(target.Height-height)/2,target.Bottom-height})
                    if(x>=.025&&x+width<=.975&&y>=.025&&y+height<=.975)candidates.Add(new Rect(x,y,width,height));
            bool Clear(Rect r)=>!occupied.Any(other=>Rect.Intersect(other,r) is var intersection&&!intersection.IsEmpty&&intersection.Width*intersection.Height>.00001);
            var clean=candidates.Where(Clear).Select(r=>new{Bounds=r,Ink=ink.Density(r)}).Where(c=>c.Ink<=.06)
                .OrderBy(c=>c.Ink).ThenBy(c=>Math.Abs(c.Bounds.Top-target.Top)).FirstOrDefault();
            if(clean is not null)bounds=clean.Bounds;
            // If the page is crowded, keep a compact marker and the complete
            // side review rather than writing over another student's answer.
            if(height>.4||clean is null)
            {
                text=$"{item.Question} {marker}"+(item.Confirmed?"":"?");bounds=new Rect(Math.Max(.005,item.X-.085),item.Y,.08,.035);
            }
            else occupied.Add(bounds);
            output.Add(new(item.X,item.Y,item.Width,item.Height,"",ReferenceHandle:handle,Kind:AiAnnotationKind.Rectangle,Style:new(color,.0015)));
            output.Add(new(bounds.X,bounds.Y,bounds.Width,bounds.Height,text,ReferenceHandle:handle,Kind:AiAnnotationKind.Text,Style:new(color,FontSize:font)){IsTeachingFeedback=true});
        }
        return output;
    }
    private sealed class InkMap
    {
        private readonly int _width,_height;private readonly int[] _sums;
        internal InkMap(BitmapSource source)
        {
            var scale=Math.Min(1,Math.Min(384d/source.PixelWidth,768d/source.PixelHeight));
            var small=new TransformedBitmap(source,new ScaleTransform(scale,scale));
            var bgra=new FormatConvertedBitmap(small,PixelFormats.Bgra32,null,0);_width=bgra.PixelWidth;_height=bgra.PixelHeight;
            var pixels=new byte[checked(_width*_height*4)];_sums=new int[checked((_width+1)*(_height+1))];
            try
            {
                bgra.CopyPixels(pixels,_width*4,0);
                for(var y=0;y<_height;y++)for(var x=0;x<_width;x++)
                {
                    var p=(y*_width+x)*4;var dark=pixels[p+3]>128&&(pixels[p]*.114+pixels[p+1]*.587+pixels[p+2]*.299)<175?1:0;var at=(y+1)*(_width+1)+x+1;
                    _sums[at]=dark+_sums[at-1]+_sums[at-(_width+1)]-_sums[at-(_width+1)-1];
                }
            }
            finally{Array.Clear(pixels);}
        }
        internal double Density(Rect bounds)
        {
            var left=Math.Clamp((int)(bounds.Left*_width),0,_width);var top=Math.Clamp((int)(bounds.Top*_height),0,_height);
            var right=Math.Clamp((int)Math.Ceiling(bounds.Right*_width),left,_width);var bottom=Math.Clamp((int)Math.Ceiling(bounds.Bottom*_height),top,_height);
            var stride=_width+1;var count=_sums[bottom*stride+right]-_sums[top*stride+right]-_sums[bottom*stride+left]+_sums[top*stride+left];
            return count/(double)Math.Max(1,(right-left)*(bottom-top));
        }
    }
}
