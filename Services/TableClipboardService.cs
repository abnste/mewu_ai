using System.Collections.Specialized;
using System.Globalization;
using System.Net;
using System.Text;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;

namespace mewu_ai_Assistant.Services;

internal static class TableClipboardService
{
    private const int MaximumTables=12;
    private const int MaximumRows=200;
    private const int MaximumColumns=32;
    private const long MaximumImagePixels=36_000_000;
    private static readonly MarkdownPipeline Pipeline=new MarkdownPipelineBuilder().UseAdvancedExtensions().Build();

    internal static IReadOnlyList<MarkdownTableData> Parse(string? markdown)
    {
        if(string.IsNullOrWhiteSpace(markdown))return [];
        var result=new List<MarkdownTableData>();
        CollectTables(Markdown.Parse(markdown,Pipeline),result);
        return result;
    }

    internal static bool TryCopy(string markdown,out int tableCount,out string? error)
    {
        var tables=Parse(markdown);tableCount=tables.Count;
        if(tables.Count==0){error="回答中没有可复制的 Markdown 表格";return false;}
        string? imagePath=null;
        try
        {
            var image=RenderImage(tables);imagePath=SaveClipboardImage(image);var data=new DataObject();
            data.SetData(DataFormats.Html,BuildHtmlClipboard(tables));
            data.SetData(DataFormats.CommaSeparatedValue,BuildCsv(tables));
            var normalizedMarkdown=BuildMarkdown(tables);data.SetData("text/markdown",normalizedMarkdown);data.SetData(DataFormats.UnicodeText,normalizedMarkdown);data.SetData(DataFormats.Text,normalizedMarkdown);
            data.SetData(DataFormats.Bitmap,image,true);data.SetFileDropList(new StringCollection{imagePath});
            if(ClipboardService.TryExecute(()=>Clipboard.SetDataObject(data,true),out error))return true;
        }
        catch(Exception ex)
        {
            new PrivacyLogger().Error("TableClipboard",ex);error=$"复制表格失败：{ex.Message}";
        }
        TryDelete(imagePath);return false;
    }

    internal static string BuildMarkdown(IReadOnlyList<MarkdownTableData> tables)
    {
        var output=new StringBuilder();
        foreach(var table in tables)
        {
            if(output.Length>0)output.AppendLine().AppendLine();var columns=table.ColumnCount;
            AppendMarkdownRow(output,table.Rows[0],columns);output.AppendLine();output.Append('|');for(var column=0;column<columns;column++)output.Append(" --- |");
            foreach(var row in table.Rows.Skip(1)){output.AppendLine();AppendMarkdownRow(output,row,columns);}
        }
        return output.ToString();
    }

    internal static string BuildCsv(IReadOnlyList<MarkdownTableData> tables)
    {
        var output=new StringBuilder();
        foreach(var table in tables)
        {
            if(output.Length>0)output.AppendLine();
            foreach(var row in table.Rows)
            {
                for(var column=0;column<table.ColumnCount;column++){if(column>0)output.Append(',');var value=column<row.Count?row[column]:string.Empty;output.Append('"').Append(value.Replace("\"","\"\"",StringComparison.Ordinal)).Append('"');}output.AppendLine();
            }
        }
        return output.ToString().TrimEnd('\r','\n');
    }

    internal static string BuildHtmlClipboard(IReadOnlyList<MarkdownTableData> tables)
    {
        var fragment=new StringBuilder("<div style=\"font-family:Calibri,Arial,sans-serif\">");
        foreach(var table in tables)
        {
            fragment.Append("<table border=\"1\" cellspacing=\"0\" cellpadding=\"4\" style=\"border-collapse:collapse;margin-bottom:12px\">");
            foreach(var (row,rowIndex) in table.Rows.Select((row,index)=>(row,index)))
            {
                fragment.Append("<tr>");var tag=rowIndex==0?"th":"td";for(var column=0;column<table.ColumnCount;column++)fragment.Append('<').Append(tag).Append('>').Append(WebUtility.HtmlEncode(column<row.Count?row[column]:string.Empty).Replace("\r\n","<br>",StringComparison.Ordinal).Replace("\n","<br>",StringComparison.Ordinal)).Append("</").Append(tag).Append('>');fragment.Append("</tr>");
            }
            fragment.Append("</table>");
        }
        fragment.Append("</div>");return WrapClipboardHtml(fragment.ToString());
    }

    private static void CollectTables(ContainerBlock container,List<MarkdownTableData> result)
    {
        foreach(var block in container)
        {
            if(result.Count>=MaximumTables)return;
            if(block is Table source)
            {
                var rows=new List<IReadOnlyList<string>>();
                foreach(var row in source.OfType<TableRow>().Take(MaximumRows))rows.Add(row.OfType<TableCell>().Take(MaximumColumns).Select(CellText).ToArray());
                if(rows.Count>0&&rows.Max(row=>row.Count)>0)result.Add(new MarkdownTableData(rows));
            }
            else if(block is ContainerBlock nested)CollectTables(nested,result);
        }
    }

    private static string CellText(TableCell cell)
    {
        var output=new StringBuilder();
        foreach(var block in cell)
        {
            if(output.Length>0)output.AppendLine();
            if(block is LeafBlock leaf&&leaf.Inline is not null)AppendInlineText(leaf.Inline.FirstChild,output);
            else if(block is LeafBlock lines)output.Append(lines.Lines.ToString());
        }
        return output.ToString().Trim();
    }

    private static void AppendInlineText(Markdig.Syntax.Inlines.Inline? inline,StringBuilder output)
    {
        for(var current=inline;current is not null;current=current.NextSibling)
        {
            switch(current)
            {
                case LiteralInline literal:output.Append(literal.Content);break;
                case CodeInline code:output.Append(code.Content);break;
                case LineBreakInline:output.AppendLine();break;
                case LinkInline link when link.IsImage:AppendInlineText(link.FirstChild,output);break;
                case ContainerInline nested:AppendInlineText(nested.FirstChild,output);break;
            }
        }
    }

    private static void AppendMarkdownRow(StringBuilder output,IReadOnlyList<string> row,int columns)
    {
        output.Append('|');for(var column=0;column<columns;column++){var value=column<row.Count?row[column]:string.Empty;output.Append(' ').Append(value.Replace("|","\\|",StringComparison.Ordinal).Replace("\r\n","<br>",StringComparison.Ordinal).Replace("\n","<br>",StringComparison.Ordinal)).Append(" |");}
    }

    private static string WrapClipboardHtml(string fragment)
    {
        const string prefix="<html><body><!--StartFragment-->";const string suffix="<!--EndFragment--></body></html>";const string template="Version:1.0\r\nStartHTML:{0:D10}\r\nEndHTML:{1:D10}\r\nStartFragment:{2:D10}\r\nEndFragment:{3:D10}\r\n";
        var emptyHeader=string.Format(CultureInfo.InvariantCulture,template,0,0,0,0);var startHtml=Encoding.UTF8.GetByteCount(emptyHeader);var startFragment=startHtml+Encoding.UTF8.GetByteCount(prefix);var endFragment=startFragment+Encoding.UTF8.GetByteCount(fragment);var endHtml=endFragment+Encoding.UTF8.GetByteCount(suffix);return string.Format(CultureInfo.InvariantCulture,template,startHtml,endHtml,startFragment,endFragment)+prefix+fragment+suffix;
    }

    private static BitmapSource RenderImage(IReadOnlyList<MarkdownTableData> tables)
    {
        const double padding=18,cellX=10,cellY=7,gap=18,fontSize=14;var typeface=new Typeface("Segoe UI");var layouts=new List<TableImageLayout>();double fullWidth=0,fullHeight=padding;
        foreach(var table in tables)
        {
            var widths=new double[table.ColumnCount];for(var column=0;column<widths.Length;column++)widths[column]=80;
            foreach(var row in table.Rows)for(var column=0;column<table.ColumnCount;column++){var text=column<row.Count?row[column]:string.Empty;var formatted=Measure(text,typeface,fontSize,260);widths[column]=Math.Clamp(Math.Max(widths[column],formatted.WidthIncludingTrailingWhitespace+cellX*2),80,280);}
            var rowHeights=new double[table.Rows.Count];for(var rowIndex=0;rowIndex<table.Rows.Count;rowIndex++){var rowHeight=0d;for(var column=0;column<table.ColumnCount;column++){var text=column<table.Rows[rowIndex].Count?table.Rows[rowIndex][column]:string.Empty;rowHeight=Math.Max(rowHeight,Measure(text,typeface,fontSize,Math.Max(10,widths[column]-cellX*2)).Height+cellY*2);}rowHeights[rowIndex]=Math.Max(30,rowHeight);}
            var width=widths.Sum();var tableHeight=rowHeights.Sum();layouts.Add(new TableImageLayout(table,widths,rowHeights,fullHeight));fullWidth=Math.Max(fullWidth,width);fullHeight+=tableHeight+gap;
        }
        fullWidth+=padding*2;fullHeight=Math.Max(1,fullHeight-gap+padding);var scale=Math.Min(1,Math.Min(6000/Math.Max(1,fullWidth),Math.Min(6000/Math.Max(1,fullHeight),Math.Sqrt(MaximumImagePixels/Math.Max(1,fullWidth*fullHeight)))));var visual=new DrawingVisual();using(var dc=visual.RenderOpen()){dc.PushTransform(new ScaleTransform(scale,scale));dc.DrawRectangle(Brushes.White,null,new Rect(0,0,fullWidth,fullHeight));foreach(var layout in layouts)DrawTable(dc,layout,padding,typeface,fontSize,cellX,cellY);dc.Pop();}var bitmap=new RenderTargetBitmap(Math.Max(1,(int)Math.Ceiling(fullWidth*scale)),Math.Max(1,(int)Math.Ceiling(fullHeight*scale)),96,96,PixelFormats.Pbgra32);bitmap.Render(visual);bitmap.Freeze();return bitmap;
    }

    private static void DrawTable(DrawingContext dc,TableImageLayout layout,double left,Typeface typeface,double fontSize,double cellX,double cellY)
    {
        var border=new Pen(new SolidColorBrush(Color.FromRgb(190,199,211)),1);border.Freeze();var header=new SolidColorBrush(Color.FromRgb(241,244,248));header.Freeze();var y=layout.Top;
        for(var rowIndex=0;rowIndex<layout.Table.Rows.Count;rowIndex++)
        {
            var x=left;for(var column=0;column<layout.Table.ColumnCount;column++){var rect=new Rect(x,y,layout.ColumnWidths[column],layout.RowHeights[rowIndex]);dc.DrawRectangle(rowIndex==0?header:Brushes.White,border,rect);var text=column<layout.Table.Rows[rowIndex].Count?layout.Table.Rows[rowIndex][column]:string.Empty;var formatted=Measure(text,typeface,fontSize,Math.Max(10,rect.Width-cellX*2));dc.DrawText(formatted,new Point(rect.X+cellX,rect.Y+cellY));x+=rect.Width;}y+=layout.RowHeights[rowIndex];
        }
    }

    private static FormattedText Measure(string text,Typeface typeface,double fontSize,double maxWidth)
    {
        var formatted=new FormattedText(text??string.Empty,CultureInfo.CurrentUICulture,FlowDirection.LeftToRight,typeface,fontSize,Brushes.Black,1){MaxTextWidth=Math.Max(1,maxWidth),TextAlignment=TextAlignment.Left};return formatted;
    }

    private static string SaveClipboardImage(BitmapSource image)
    {
        var directory=ClipboardService.FileDropStagingDirectory;Directory.CreateDirectory(directory);if(new DirectoryInfo(directory).Attributes.HasFlag(FileAttributes.ReparsePoint))throw new InvalidOperationException("剪贴板媒体目录不能是文件系统链接");ClipboardService.CleanupStagedFiles(ClipboardService.DefaultStagingMaxAge,directory);var path=Path.Combine(directory,$"表格_{DateTime.Now:yyyyMMdd_HHmmss_fff}.png");try{var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var stream=new FileStream(path,FileMode.CreateNew,FileAccess.Write,FileShare.Read);encoder.Save(stream);return path;}catch{TryDelete(path);throw;}
    }

    private static void TryDelete(string? path){if(string.IsNullOrWhiteSpace(path))return;try{if(File.Exists(path))File.Delete(path);}catch{}}
    private sealed record TableImageLayout(MarkdownTableData Table,double[] ColumnWidths,double[] RowHeights,double Top);
}

internal sealed record MarkdownTableData(IReadOnlyList<IReadOnlyList<string>> Rows)
{
    internal int ColumnCount=>Rows.Count==0?0:Rows.Max(row=>row.Count);
}
