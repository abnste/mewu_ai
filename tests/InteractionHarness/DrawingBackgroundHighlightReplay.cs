// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Point = System.Windows.Point;
using Size = System.Windows.Size;
using Panel = System.Windows.Controls.Panel;
using Color = System.Windows.Media.Color;
using FlowDirection = System.Windows.FlowDirection;
using Application = System.Windows.Application;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Interop;
using System.Security.Cryptography;

internal static class DrawingBackgroundHighlightReplay
{
    internal static void RunVisual()
    {
        var directory = ReplayOutputDirectory.PrepareWorkingDirectory("background-highlight-visual");
        PrivacyLogger.ConfigureIsolatedReplayDirectory(Path.Combine(directory, "logs"));
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/MewuAI;component/Themes/LightTheme.xaml", UriKind.Relative) });
        using var host = new AppHost(app, null, "MewuAI-HighlightFringe-" + Guid.NewGuid().ToString("N"));
        host.Settings.EnableVoiceInput = false; host.Settings.AutomaticallyStartListening = false; host.Settings.SaveConversationHistory = false;
        var checks = new List<string>(); var failures = new List<string>(); var evidence = new List<object>();
        void Check(bool condition, string label) { checks.Add(label); if (!condition) failures.Add(label); }
        CaptureOverlayWindow? overlay = null;
        const int logicalWidth = 720, rowHeight = 64;
        var rows = new[] {
            (Background: Color.FromRgb(243,243,244), Text: Colors.Black, Size:22d),
            (Background: Color.FromRgb(230,218,192), Text: Colors.Black, Size:22d),
            (Background: Color.FromRgb(32,36,42), Text: Colors.White, Size:22d),
            (Background: Color.FromRgb(238,244,250), Text: Color.FromRgb(36,78,210), Size:22d),
            (Background: Color.FromRgb(243,243,244), Text: Color.FromRgb(180,180,180), Size:12d),
            (Background: Color.FromRgb(243,243,244), Text: Colors.Black, Size:10d) };
        try
        {
            foreach (var scale in new[] { 1d, 1.75d, 2d })
            {
                var width = (int)(logicalWidth * scale); var height = (int)(rowHeight * rows.Length * scale);
                var glyphVisual = new DrawingVisual();
                using (var drawing = glyphVisual.RenderOpen())
                {
                    drawing.PushTransform(new ScaleTransform(scale, scale));
                    for (var i = 0; i < rows.Length; i++)
                    {
                        var row = rows[i];
                        var text = new FormattedText("重点高亮保护正文，边缘平滑 Clear text 123", CultureInfo.InvariantCulture, FlowDirection.LeftToRight,
                            new Typeface("Microsoft YaHei UI"), row.Size, new SolidColorBrush(row.Text), scale);
                        drawing.DrawText(text, new Point(32, i * rowHeight + 14));
                    }
                    drawing.Pop();
                }
                var glyph = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); glyph.Render(glyphVisual);
                var coverage = Pixels(glyph); var sourcePixels = new byte[width * height * 4];
                for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
                {
                    var row = rows[Math.Min(rows.Length - 1, (int)(y / scale) / rowHeight)]; var at = (y * width + x) * 4; var inverse = 255 - coverage[at + 3];
                    byte[] background = [row.Background.B, row.Background.G, row.Background.R];
                    for (var c = 0; c < 3; c++) sourcePixels[at + c] = (byte)Math.Min(255, coverage[at + c] + (background[c] * inverse + 127) / 255);
                    sourcePixels[at + 3] = 255;
                }
                var source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, sourcePixels, width * 4); source.Freeze();
                var protection = BackgroundHighlightService.CreateSource(source, scale);
                var strokes = Enumerable.Range(0, rows.Length).Select(i => new BackgroundHighlightStroke(
                    new StylusPointCollection { new StylusPoint(12 * scale, (i * rowHeight + 32) * scale), new StylusPoint(708 * scale, (i * rowHeight + 32) * scale) },
                    new DrawingAttributes { Color = Colors.Red, Width = 52 * scale, Height = 52 * scale, FitToCurve = false }, protection, new Rect(0,0,width,height))).ToArray();
                BitmapSource RenderStrokes(int count)
                {
                    var visual = new DrawingVisual(); using (var drawing = visual.RenderOpen())
                    { drawing.DrawImage(source,new Rect(0,0,width,height)); for(var repeat=0;repeat<count;repeat++) foreach(var stroke in strokes) stroke.Draw(drawing); }
                    var output = new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32); output.Render(visual); output.Freeze(); return output;
                }
                var once = RenderStrokes(1); var twice = RenderStrokes(2); var threeTimes = RenderStrokes(3);
                overlay = new CaptureOverlayWindow(host,null,new CaptureFrame(0,0,source));
                const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
                object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name,flags)!.Invoke(overlay,args);
                var root = (Canvas)overlay.FindName("Root"); root.Width=logicalWidth; root.Height=rowHeight*rows.Length;
                root.Measure(new Size(root.Width,root.Height));root.Arrange(new Rect(0,0,root.Width,root.Height));root.UpdateLayout();
                var item=Invoke("CreateSelection",false)!; var itemType=item.GetType();
                ((IList)typeof(CaptureOverlayWindow).GetField("_selections",flags)!.GetValue(overlay)!).Add(item);
                itemType.GetField("Bounds")!.SetValue(item,new Rect(0,0,logicalWidth,rowHeight*rows.Length)); Invoke("UpdateSelection",item);
                typeof(CaptureOverlayWindow).GetField("_drawColor",flags)!.SetValue(overlay,Colors.Red);
                BitmapSource Saved() => (BitmapSource)Invoke("RenderSelectionImage",item,true,false,false)!;
                void AddMarks() { for(var i=0;i<rows.Length;i++) Invoke("AddRegionMark",item,new Rect(10,i*rowHeight+6,700,52)); }
                AddMarks(); var regionOnce=Saved(); AddMarks();var regionTwice=Saved(); AddMarks();var regionThreeTimes=Saved();
                var prefix="highlight-red-"+(int)(96*scale);
                Save(source,Path.Combine(directory,prefix+"-before.png"));
                foreach(var variant in new[]{(Name:"stroke",Image:once),(Name:"stroke-twice",Image:twice),(Name:"stroke-three-times",Image:threeTimes),(Name:"region",Image:regionOnce),(Name:"region-twice",Image:regionTwice),(Name:"region-three-times",Image:regionThreeTimes)})
                {
                    Save(variant.Image,Path.Combine(directory,prefix+"-"+variant.Name+".png"));
                    Check(variant.Image.PixelWidth==width&&variant.Image.PixelHeight==height,prefix+variant.Name+" retains native pixel dimensions");
                    var actual=Pixels(variant.Image);
                    for(var i=0;i<rows.Length;i++)
                    {
                        var core=0;var coreError=0;var adjacentAa=0;var tintedAa=0;var ringPixels=0;var maxRing=0;
                        var tintedBackground=Pixel(variant.Image,new Point((int)(690*scale),(int)((i*rowHeight+32)*scale)));
                        byte[] foreground=[rows[i].Text.B,rows[i].Text.G,rows[i].Text.R];
                        for(var y=(int)((i*rowHeight+10)*scale);y<(int)((i*rowHeight+51)*scale);y++)
                        for(var x=(int)(28*scale);x<(int)(650*scale);x++)
                        {
                            var at=(y*width+x)*4;var alpha=coverage[at+3]; if(alpha==0)continue;
                            if(alpha==255){core++;for(var c=0;c<3;c++)coreError=Math.Max(coreError,Math.Abs(actual[at+c]-sourcePixels[at+c]));continue;}
                            var nearCore=false;
                            for(var dy=-1;dy<=1;dy++)for(var dx=-1;dx<=1;dx++)if(coverage[((y+dy)*width+x+dx)*4+3]==255)nearCore=true;
                            if(!nearCore)continue; adjacentAa++; if(Enumerable.Range(0,3).Any(c=>actual[at+c]!=sourcePixels[at+c]))tintedAa++;
                            var excess=0;for(var c=0;c<3;c++)excess=Math.Max(excess,Math.Max(Math.Min(foreground[c],tintedBackground[c])-actual[at+c],actual[at+c]-Math.Max(foreground[c],tintedBackground[c])));
                            if(excess>2)ringPixels++;maxRing=Math.Max(maxRing,excess);
                        }
                        var label=prefix+" "+variant.Name+" row"+i;
                        Check(core>20&&coreError==0,label+" opaque text cores remain exact");
                        Check(adjacentAa>20&&tintedAa>adjacentAa/2,label+" real antialias edges receive smooth tint");
                        Check(ringPixels==0,label+" edges do not overshoot tinted background/core color range");
                        var originalBackground = Pixel(source,new Point((int)(690*scale),(int)((i*rowHeight+32)*scale)));
                        Check(!tintedBackground.SequenceEqual(originalBackground),label+" blank background actually receives red tint");
                        evidence.Add(new{scale,variant=variant.Name,row=i,fontSize=rows[i].Size,core,coreError,adjacentAa,tintedAa,ringPixels,maxRing});
                    }
                }
                var comparison=new DrawingVisual();using(var drawing=comparison.RenderOpen())
                {drawing.DrawImage(source,new Rect(0,0,width,height));drawing.DrawImage(once,new Rect(0,height+16,width,height));drawing.DrawImage(regionOnce,new Rect(0,2*(height+16),width,height));}
                var combined=new RenderTargetBitmap(width,height*3+32,96,96,PixelFormats.Pbgra32);combined.Render(comparison);Save(combined,Path.Combine(directory,prefix+"-before-stroke-region.png"));
                Check(Pixels(source).SequenceEqual(sourcePixels),prefix+" source bytes unchanged");
                Check(new WindowInteropHelper(overlay).Handle==IntPtr.Zero&&!overlay.IsVisible&&!overlay.IsLoaded,prefix+" no HWND or desktop input");
                overlay.Close();overlay=null;
            }
        }
        catch(Exception error){failures.Add(error.ToString());}
        finally{overlay?.Close();app.Shutdown();}
        static string Hash(string path){using var input=File.OpenRead(path);return Convert.ToHexString(SHA256.HashData(input));}
        File.WriteAllText(Path.Combine(directory,"result.json"),JsonSerializer.Serialize(new{passed=failures.Count==0,checks,evidence,failures,productSha256=Hash(typeof(BackgroundHighlightService).Assembly.Location),harnessSha256=Hash(typeof(DrawingBackgroundHighlightReplay).Assembly.Location),syntheticOnly=true,shownWindows=false,productionSettingsUsed=false,clipboardUsed=false},new JsonSerializerOptions{WriteIndented=true}));
        Environment.ExitCode=failures.Count==0?0:1;
    }

    internal static void Verify(CaptureOverlayWindow overlay, object item, Action<bool, string> require, string directory)
    {
        const BindingFlags flags = BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public;
        object? Invoke(string name, params object?[] args) => typeof(CaptureOverlayWindow).GetMethod(name, flags)!.Invoke(overlay, args);
        object? Field(string name) => typeof(CaptureOverlayWindow).GetField(name, flags)!.GetValue(overlay);
        void SetField(string name, object value) => typeof(CaptureOverlayWindow).GetField(name, flags)!.SetValue(overlay, value);
        T Property<T>(object value, string name) => (T)value.GetType().GetProperty(name, flags)!.GetValue(value)!;
        var canvas = Property<InkCanvas>(item, "Markup"); var order = Property<IList>(item, "DrawingOrder");
        var snapshot = Invoke("CaptureOverlaySnapshot")!; var originalFrame = (CaptureFrame)Field("_frame")!; var originalWidth = (double)Field("_drawHighlightWidth")!; var originalMoving = (bool)Field("_moving")!;
        var crop = (Rect)item.GetType().GetField("Bounds")!.GetValue(item)!;
        BitmapSource Saved(bool manual = true) => (BitmapSource)Invoke("RenderSelectionImage", item, manual, false, false)!;
        void Highlight(double y, bool captureLoss = false, double end = 220)
        {
            Invoke("DrawHighlight", overlay, new RoutedEventArgs());
            require((bool)Invoke("BeginDrawingGesture", item, new Point(100, y), canvas, 1, canvas)!, "background highlighter uses explicit custom gesture dispatcher");
            Invoke("UpdateBackgroundHighlightPreview", item, new Point(end, y));
            if (!captureLoss) Invoke("CommitBackgroundHighlightPreview", item, new Point(end, y));
            canvas.ReleaseMouseCapture();
        }
        try
        {
            Invoke("DrawClear", overlay, new RoutedEventArgs()); SetField("_drawHighlightWidth", 30d); Invoke("SetDrawColor", Colors.Yellow);
            var pixels = (Int32Rect)Invoke("ToPixelRect", crop)!; var sx = pixels.Width / crop.Width; var sy = pixels.Height / crop.Height;
            var width = originalFrame.Image.PixelWidth; var height = originalFrame.Image.PixelHeight; var bytes = new byte[width * height * 4];
            for (var y = 0; y < height; y++) for (var x = 0; x < width; x++)
            {
                var px = (x - pixels.X) / sx; var py = (y - pixels.Y) / sy; var dark = py >= 130 && py < 170; var at = (y * width + x) * 4;
                byte[] color = dark ? [20, 30, 40] : [245, 245, 245];
                var inGlyph = px >= 140 && px < 144 && (py >= 55 && py < 85 || py >= 135 && py < 165 || py >= 215 && py < 245);
                if (inGlyph) color = py < 100 ? [0, 0, 0] : py < 200 ? [255, 255, 255] : [190, 75, 70];
                bytes[at] = color[0]; bytes[at + 1] = color[1]; bytes[at + 2] = color[2]; bytes[at + 3] = 255;
            }
            BitmapSource source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, bytes, width * 4); source.Freeze();
            var textCases = new[] { (Y: 300d, Background: Colors.White, Foreground: Colors.Black),
                (Y: 370d, Background: Color.FromRgb(32, 36, 42), Foreground: Colors.White),
                (Y: 440d, Background: Color.FromRgb(238, 244, 250), Foreground: Color.FromRgb(36, 78, 210)) };
            var spaceSamples = new List<(double Row, Point Pixel)>();
            var textVisual = new DrawingVisual(); var glyphVisual = new DrawingVisual();
            using (var drawing = textVisual.RenderOpen())
            using (var glyphDrawing = glyphVisual.RenderOpen())
            {
                drawing.DrawImage(source, new Rect(0, 0, width, height));
                drawing.PushTransform(new MatrixTransform(sx, 0, 0, sy, pixels.X, pixels.Y));
                glyphDrawing.PushTransform(new MatrixTransform(sx, 0, 0, sy, pixels.X, pixels.Y));
                foreach (var row in textCases)
                {
                    drawing.DrawRectangle(new SolidColorBrush(row.Background), null, new Rect(70, row.Y - 25, 760, 50));
                    var text = new FormattedText("重点高亮 Clear text 123", CultureInfo.InvariantCulture, FlowDirection.LeftToRight,
                        new Typeface("Microsoft YaHei UI"), 22, new SolidColorBrush(row.Foreground), 1);
                    glyphDrawing.DrawText(text, new Point(100, row.Y - 17));
                    foreach (var index in new[] { 4, 10, 15 })
                    {
                        var space = text.BuildHighlightGeometry(new Point(100, row.Y - 17), index, 1).Bounds;
                        spaceSamples.Add((row.Y, new Point((space.Left + space.Width / 2) * sx, (space.Top + space.Height / 2) * sy)));
                    }
                }
                drawing.Pop();
                glyphDrawing.Pop();
            }
            var baseSource = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); baseSource.Render(textVisual);
            var glyphSource = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); glyphSource.Render(glyphVisual);
            var opaquePixels = Pixels(baseSource); var glyphLayer = Pixels(glyphSource);
            // A real desktop capture is opaque. Composite actual WPF glyph coverage
            // over the known opaque backdrop using integer source-over, avoiding
            // RTB's alpha=254 rounding of background pixels beside dark-row text.
            for (var at = 0; at < opaquePixels.Length; at += 4)
            {
                var inverseAlpha = 255 - glyphLayer[at + 3];
                for (var channel = 0; channel < 3; channel++) opaquePixels[at + channel] = (byte)Math.Min(255, glyphLayer[at + channel] + (opaquePixels[at + channel] * inverseAlpha + 127) / 255);
                opaquePixels[at + 3] = 255;
            }
            source = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgra32, null, opaquePixels, width * 4); source.Freeze();
            SetField("_frame", new CaptureFrame(originalFrame.OriginX, originalFrame.OriginY, source)); Invoke("UpdateSelection", item);
            var clean = Saved(false); var before = order.Count; var sourceBytes = Pixels(source);
            var initialSource = Invoke("GetBackgroundHighlightSource", item)!;
            Highlight(70); require(canvas.Strokes.Count == 1 && order.Count == before + 1 && canvas.Strokes[0] is BackgroundHighlightStroke,
                "one real highlighter release commits exactly one custom stroke");
            require(ReferenceEquals(initialSource, Invoke("GetBackgroundHighlightSource", item)), "committing/moving highlighter points reuses unchanged source-mask cache");
            var first = Saved(); var black = new Point(142 * sx, 70 * sy); var background = new Point(110 * sx, 70 * sy);
            require(Pixel(first, black).SequenceEqual(Pixel(clean, black)), "non-origin freehand highlighter leaves black text unchanged at its actual source coordinates");
            require(!Pixel(first, background).SequenceEqual(Pixel(clean, background)), "freehand highlighter visibly tints the nearby white background");
            var live = RenderCanvas(canvas, clean.PixelWidth, clean.PixelHeight);
            require(Pixel(live, black)[3] == 0 && Pixel(live, background)[3] > 0, "actual live InkCanvas has transparent protected text and visible background tint");
            Highlight(70, captureLoss: true);
            require(canvas.Strokes.Count == 2 && order.Count == before + 2, "native capture loss commits a highlighter once without duplicate StrokeCollected history");
            require(Pixel(Saved(), black).SequenceEqual(Pixel(clean, black)), "two overlapping highlighters still leave black text unchanged");
            Highlight(150); Highlight(230);
            foreach (var y in new[] { 150d, 230d }) require(Pixel(Saved(), new Point(142 * sx, y * sy)).SequenceEqual(Pixel(clean, new Point(142 * sx, y * sy))),
                "highlighter protects light or colored text at row " + y);
            var committed = order.Count; var strokeCount = canvas.Strokes.Count;
            Invoke("DrawHighlight", overlay, new RoutedEventArgs()); Invoke("BeginDrawingGesture", item, new Point(100, 310), canvas, 1, canvas);
            Invoke("UpdateBackgroundHighlightPreview", item, new Point(220, 310)); Invoke("CancelBackgroundHighlightPreview"); canvas.ReleaseMouseCapture();
            require(canvas.Strokes.Count == strokeCount && order.Count == committed, "canceling a highlighter preview records neither stroke nor history");

            Invoke("AddRegionMark", item, new Rect(100, 50, 140, 40)); Invoke("AddRegionMark", item, new Rect(100, 130, 140, 40)); Invoke("AddRegionMark", item, new Rect(100, 210, 140, 40));
            var marked = Saved();
            foreach (var y in new[] { 70d, 150d, 230d }) require(Pixel(marked, new Point(142 * sx, y * sy)).SequenceEqual(Pixel(clean, new Point(142 * sx, y * sy))),
                "overlapping emphasis and highlighter protect source text at row " + y);
            require(!Pixel(marked, new Point(110 * sx, 70 * sy)).SequenceEqual(Pixel(first, background)), "emphasis adds visible background tint independently of protected text");
            Save(marked, Path.Combine(directory, "background-highlights-three-text-colors.png"));
            var baseline = Pixels(marked); var retained = Invoke("CaptureOverlaySnapshot")!;
            var beforeDragSource = Invoke("GetBackgroundHighlightSource", item); SetField("_moving", true);
            for (var step = 1; step <= 6; step++)
            {
                Invoke("SetSelectionBoundsPreservingManualContent", item, new Rect(crop.X + step * 5, crop.Y + step * 3, crop.Width, crop.Height)); Invoke("UpdateSelection", item);
                require(ReferenceEquals(beforeDragSource, Invoke("GetBackgroundHighlightSource", item)), "reframe pointer preview reuses the same frozen protection mask " + step);
            }
            Invoke("SetSelectionBoundsPreservingManualContent", item, new Rect(crop.X + 35, crop.Y + 25, crop.Width, crop.Height)); Invoke("UpdateSelection", item);
            require(ReferenceEquals(beforeDragSource, Invoke("GetBackgroundHighlightSource", item)), "last preview retains source identity before pointer release");
            SetField("_moving", false); Invoke("RefreshBackgroundHighlightSources", item);
            var afterDragSource = Invoke("GetBackgroundHighlightSource", item);
            require(!ReferenceEquals(beforeDragSource, afterDragSource) && ReferenceEquals(afterDragSource, Invoke("GetBackgroundHighlightSource", item)), "completed reframe computes one new source and then reuses it");
            require(Pixel(Saved(), new Point((142 - 35) * sx, (70 - 25) * sy)).SequenceEqual(Pixel(clean, black)), "reframing rebases highlighter source masks without shifting protection away from text");
            Invoke("SetSelectionBoundsPreservingManualContent", item, crop); Invoke("UpdateSelection", item);
            require(Pixels(Saved()).SequenceEqual(baseline), "reframe away/back restores exact background-highlight composition");
            Invoke("DrawClear", overlay, new RoutedEventArgs()); Invoke("DrawUndo", overlay, new RoutedEventArgs());
            require(canvas.Strokes.OfType<BackgroundHighlightStroke>().Count() == 4 && Pixels(Saved()).SequenceEqual(baseline), "Clear undo restores custom stroke types and protected-pixel rendering");
            Invoke("ApplyOverlaySnapshot", retained);
            require(canvas.Strokes.OfType<BackgroundHighlightStroke>().Count() == 4 && Pixels(Saved()).SequenceEqual(baseline), "snapshot cloning preserves background-only stroke rendering");
            require(Pixels(source).SequenceEqual(sourceBytes), "background masks and their history never modify original screenshot bytes");

            var glyphEvidence = new List<object>(); var backgroundFailures = new List<string>(); var glyphFailures = new List<string>();
            foreach (var row in textCases)
            {
                Highlight(row.Y, end: 780); Invoke("AddRegionMark", item, new Rect(90, row.Y - 20, 710, 40));
                var actual = Saved(); var referencePixels = Pixels(clean); var actualPixels = Pixels(actual); var glyphPixels = 0; var maximumError = 0; var differentGlyphPixels = 0; var antialiasPixels = 0; var tintedAntialiasPixels = 0; var differences = new List<object>();
                var back = new byte[] { row.Background.B, row.Background.G, row.Background.R };
                for (var y = (int)((row.Y - 19) * sy); y < (int)((row.Y + 20) * sy); y++)
                for (var x = (int)(95 * sx); x < (int)(710 * sx); x++)
                {
                    var at = (y * clean.PixelWidth + x) * 4;
                    var glyphAlpha = glyphLayer[((y + pixels.Y) * width + x + pixels.X) * 4 + 3];
                    if (glyphAlpha == 0) continue;
                    if (glyphAlpha < 255)
                    {
                        antialiasPixels++;
                        if (Enumerable.Range(0, 3).Any(channel => actualPixels[at + channel] != referencePixels[at + channel])) tintedAntialiasPixels++;
                        continue;
                    }
                    glyphPixels++;
                    for (var channel = 0; channel < 3; channel++) maximumError = Math.Max(maximumError, Math.Abs(actualPixels[at + channel] - referencePixels[at + channel]));
                    if (Enumerable.Range(0, 3).Any(channel => actualPixels[at + channel] != referencePixels[at + channel]))
                    {
                        differentGlyphPixels++;
                        if (differences.Count < 16) differences.Add(new { x, y, before = referencePixels.AsSpan(at, 4).ToArray(), after = actualPixels.AsSpan(at, 4).ToArray() });
                    }
                }
                if (glyphPixels <= 100 || maximumError != 0 || antialiasPixels == 0 || tintedAntialiasPixels <= antialiasPixels / 4)
                    glyphFailures.Add(row.Y + ":corePixels=" + glyphPixels + ":different=" + differentGlyphPixels + ":maxError=" + maximumError + ":AA=" + antialiasPixels + ":tintedAA=" + tintedAntialiasPixels);
                require(!Pixel(actual, new Point(750 * sx, row.Y * sy)).SequenceEqual(Pixel(clean, new Point(750 * sx, row.Y * sy))), "blank background beyond actual text is tinted at row " + row.Y);
                var spaces = spaceSamples.Where(sample => sample.Row == row.Y).Select(sample => sample.Pixel).ToArray();
                var counters = CounterSamples(clean, new Int32Rect((int)(95 * sx), (int)((row.Y - 20) * sy), (int)(620 * sx), (int)(40 * sy)), back);
                require(counters.Count > 0, "real rendered bilingual font supplies enclosed background counters at row " + row.Y);
                var blankEvidence = new List<object>();
                foreach (var sample in spaces.Select(point => (Kind: "space", Point: point)).Concat(counters.Select(point => (Kind: "counter", Point: point))))
                {
                    var beforePixel = Pixel(clean, sample.Point); var afterPixel = Pixel(actual, sample.Point);
                    require(beforePixel.Take(3).SequenceEqual(back), "blank glyph sample actually contains source background");
                    var tinted = !afterPixel.SequenceEqual(beforePixel);
                    if (!tinted) backgroundFailures.Add(row.Y + ":" + sample.Kind + ":" + sample.Point);
                    blankEvidence.Add(new { kind = sample.Kind, x = sample.Point.X, y = sample.Point.Y, beforePixel, afterPixel, tinted });
                }
                glyphEvidence.Add(new { row = row.Y, glyphPixels, maximumError, differentGlyphPixels, antialiasPixels, tintedAntialiasPixels, differences, blankEvidence });
            }
            Save(Saved(), Path.Combine(directory, "background-highlights-real-bilingual-text.png"));
            File.WriteAllText(Path.Combine(directory, "background-highlight-real-text-pixels.json"), JsonSerializer.Serialize(glyphEvidence, new JsonSerializerOptions { WriteIndented = true }));
            require(glyphFailures.Count == 0, "actual bilingual opaque glyph cores remain unchanged while antialias edges are tinted: " + string.Join(";", glyphFailures));
            require(backgroundFailures.Count == 0, "all real word spaces and enclosed glyph counters are tinted: " + string.Join(";", backgroundFailures));

            var largePixels = Enumerable.Repeat((byte)255, 3840 * 2160 * 4).ToArray();
            for (var y = 1040; y < 1080; y++) for (var x = 1900; x < 1905; x++) { var at = (y * 3840 + x) * 4; largePixels[at] = largePixels[at + 1] = largePixels[at + 2] = 0; }
            var large = BitmapSource.Create(3840, 2160, 96, 96, PixelFormats.Bgra32, null, largePixels, 3840 * 4); large.Freeze();
            var watch = Stopwatch.StartNew(); var largeMask = BackgroundHighlightService.CreateSource(large, 1.75); var milliseconds = watch.Elapsed.TotalMilliseconds;
            require(largeMask.Mask.IsFrozen && Pixel(largeMask.Mask, new Point(1902, 1060))[3] == 0, "4K protection-mask sample preserves the synthetic thin glyph");
            File.WriteAllText(Path.Combine(directory, "background-highlight-timing.json"), JsonSerializer.Serialize(new { width = 3840, height = 2160, milliseconds, unchangedSourceCacheReused = true }));
        }
        finally
        {
            Invoke("CancelBackgroundHighlightPreview"); if (canvas.IsMouseCaptured) canvas.ReleaseMouseCapture();
            SetField("_moving", originalMoving); SetField("_drawHighlightWidth", originalWidth); SetField("_frame", originalFrame); Invoke("ApplyOverlaySnapshot", snapshot);
        }
    }
    private static List<Point> CounterSamples(BitmapSource image, Int32Rect area, byte[] background)
    {
        var bytes = new byte[area.Width * area.Height * 4]; image.CopyPixels(area, bytes, area.Width * 4, 0); var visited = new bool[area.Width * area.Height];
        bool IsBackground(int x, int y) => x >= 0 && y >= 0 && x < area.Width && y < area.Height &&
            bytes[(y * area.Width + x) * 4] == background[0] && bytes[(y * area.Width + x) * 4 + 1] == background[1] && bytes[(y * area.Width + x) * 4 + 2] == background[2];
        var samples = new List<Point>();
        for (var y = 0; y < area.Height; y++) for (var x = 0; x < area.Width; x++)
        {
            var start = y * area.Width + x; if (visited[start] || !IsBackground(x, y)) continue;
            var queue = new Queue<(int X, int Y)>(); var component = new List<(int X, int Y)>(); var boundary = false; queue.Enqueue((x, y)); visited[start] = true;
            while (queue.TryDequeue(out var point))
            {
                component.Add(point); if (point.X == 0 || point.Y == 0 || point.X == area.Width - 1 || point.Y == area.Height - 1) boundary = true;
                foreach (var next in new[] { (X: point.X - 1, Y: point.Y), (X: point.X + 1, Y: point.Y), (X: point.X, Y: point.Y - 1), (X: point.X, Y: point.Y + 1) })
                { if (!IsBackground(next.X, next.Y)) continue; var index = next.Y * area.Width + next.X; if (visited[index]) continue; visited[index] = true; queue.Enqueue(next); }
            }
            if (boundary) continue;
            var candidates = component.Where(point => Enumerable.Range(-1, 3).All(dy => Enumerable.Range(-1, 3).All(dx => IsBackground(point.X + dx, point.Y + dy)))).ToArray();
            if (candidates.Length == 0) continue;
            var cx = component.Average(point => point.X); var cy = component.Average(point => point.Y);
            var chosen = candidates.OrderBy(point => Math.Pow(point.X - cx, 2) + Math.Pow(point.Y - cy, 2)).First(); samples.Add(new Point(area.X + chosen.X, area.Y + chosen.Y));
        }
        return samples;
    }
    private static BitmapSource RenderCanvas(InkCanvas canvas, int width, int height)
    {
        var parent = (Panel)VisualTreeHelper.GetParent(canvas); var index = parent.Children.IndexOf(canvas); var size = new Size(canvas.Width, canvas.Height);
        var host = new Grid { Width = size.Width, Height = size.Height }; parent.Children.Remove(canvas);
        try { host.Children.Add(canvas); host.Measure(size); host.Arrange(new Rect(size)); host.UpdateLayout(); var image = new RenderTargetBitmap(width, height, 96 * width / size.Width, 96 * height / size.Height, PixelFormats.Pbgra32); image.Render(host); image.Freeze(); return image; }
        finally { host.Children.Remove(canvas); parent.Children.Insert(index, canvas); parent.UpdateLayout(); }
    }
    private static byte[] Pixel(BitmapSource image, Point point) { var pixel = new byte[4]; image.CopyPixels(new Int32Rect((int)point.X, (int)point.Y, 1, 1), pixel, 4, 0); return pixel; }
    private static byte[] Pixels(BitmapSource source) { var bytes = new byte[source.PixelWidth * source.PixelHeight * 4]; source.CopyPixels(bytes, source.PixelWidth * 4, 0); return bytes; }
    private static void Save(BitmapSource image, string path) { var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(image)); using var output = File.Create(path); encoder.Save(output); }
}
