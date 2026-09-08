// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Image=System.Windows.Controls.Image;

internal static class CaptureConnectionReplay
{
    private const BindingFlags Flags=BindingFlags.Instance|BindingFlags.NonPublic|BindingFlags.DeclaredOnly;
    internal static void Run(Application app,CaptureOverlayWindow overlay)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;overlay.Show();
        app.Dispatcher.BeginInvoke(DispatcherPriority.Normal,new Action(async()=>
        {
            var checks=new List<string>();string? failure=null;
            try
            {
                var root=(Canvas)overlay.FindName("Root");var w=root.ActualWidth;var h=root.ActualHeight;
                var a=Add(new Rect(w*.07,h*.15,w*.30,h*.4));var b=Add(new Rect(w*.6,h*.22,w*.3,h*.4));
                string Handle(object item)=>(string)item.GetType().GetProperty("ReferenceHandle")!.GetValue(item)!;
                var sourceList=(IList)Get("_lastSentSelections");sourceList.Add(a);sourceList.Add(b);
                var sent=(IList)Get("_lastSentAnnotationTargets");var targetType=typeof(CaptureOverlayWindow).GetNestedType("SentAnnotationTarget",BindingFlags.NonPublic)!;
                foreach(var item in new[]{a,b})sent.Add(Activator.CreateInstance(targetType,Handle(item),AiAttachmentType.Image,item));
                var link=new AiAnnotation(.15,.25,.3,.25,"两个截图里的同一项",ReferenceHandle:Handle(a),Kind:AiAnnotationKind.Connection,Destination:new(1,Handle(b),.35,.3,.25,.2));
                var task=(Task)Invoke("MapAnnotationsAsync",new AiAnnotation[]{link},CancellationToken.None)!;await task;
                Invoke("ApplyAnnotationMapping",task.GetType().GetProperty("Result")!.GetValue(task)!,AiAnnotationUpdateMode.Replace,false);
                Require(Count()==1,"Connection did not survive actual mapping");checks.Add("actual-parser-target-mapping");
                Invoke("Select",0);Invoke("ShowToolbar");await Task.Delay(120);overlay.UpdateLayout();
                var drawing=(Image)overlay.FindName("CrossRegionConnections");
                Require(drawing.Source is DrawingImage&&drawing.IsHitTestVisible==false,"Connection is missing or intercepts tools");checks.Add("independent-noninteractive-connection-layer");
                var clean=(BitmapSource)Invoke("RenderSelectionImage",b,false,false,false)!;var annotated=(BitmapSource)Invoke("RenderSelectionImage",b,true,true,true)!;
                Require(!Pixels(clean).SequenceEqual(Pixels(annotated)),"Destination export lost the incoming connection");checks.Add("incoming-endpoint-export");
                var before=Invoke("CaptureOverlaySnapshot")!;
                Invoke("Select",1);Invoke("RemoveActiveSelection",true);Invoke("RecordOverlayOperation",before,"删除截图区域");
                Require(Count()==0,"Removed target left a dangling connection");checks.Add("delete-cleans-connection");
                Invoke("UndoOverlayOperation");Require(Count()==1,"Undo did not restore the connection");checks.Add("undo-restores-both-endpoints");
                Invoke("RedoOverlayOperation");Require(Count()==0,"Redo restored a dangling connection");Invoke("UndoOverlayOperation");checks.Add("redo-removes-connection");
                await Task.Delay(120);overlay.UpdateLayout();
                // All background pixels were synthesized by the harness.
                var bitmap=new RenderTargetBitmap((int)Math.Ceiling(w),(int)Math.Ceiling(h),96,96,PixelFormats.Pbgra32);bitmap.Render(root);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using(var file=File.Create(Path.GetFullPath(".codex-build/connections-preview.png")))encoder.Save(file);
            }
            catch(Exception ex){failure=ex.ToString();Environment.ExitCode=1;}
            finally
            {
                overlay.Close();File.WriteAllText(Path.GetFullPath(".codex-build/connections-result.json"),JsonSerializer.Serialize(new{checks,failure}),new System.Text.UTF8Encoding(false));app.Shutdown(Environment.ExitCode);
            }
            object Get(string name)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.GetValue(overlay)!;
            object? Invoke(string name,params object[] args)=>typeof(CaptureOverlayWindow).GetMethod(name,Flags)!.Invoke(overlay,args);
            int Count()=>((IEnumerable)Invoke("GetCrossRegionConnections")!).Cast<object>().Count();
            object Add(Rect bounds){var item=Invoke("CreateSelection",false)!;item.GetType().GetField("Bounds")!.SetValue(item,bounds);((IList)Get("_selections")).Add(item);Invoke("UpdateSelection",item);return item;}
        }));
    }
    private static byte[] Pixels(BitmapSource source){var frame=new FormatConvertedBitmap(source,PixelFormats.Bgra32,null,0);var bytes=new byte[frame.PixelWidth*frame.PixelHeight*4];frame.CopyPixels(bytes,frame.PixelWidth*4,0);return bytes;}
    private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
}
