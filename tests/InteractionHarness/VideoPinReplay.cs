// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Ink;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;

internal static class VideoPinReplay
{
    private const BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static async Task VerifyAsync(CaptureOverlayWindow overlay,object item,string source)
    {
        var log=Path.GetFullPath(".codex-build/video-pin-stages.txt");
        File.WriteAllText(log,"start\n");
        var hash=SHA256.HashData(await File.ReadAllBytesAsync(source));
        typeof(CaptureOverlayWindow).GetMethod("ToggleVideoPlayback",Private)!.Invoke(overlay,new object[]{overlay,new RoutedEventArgs()});
        for(var attempt=0;attempt<12;attempt++)
        {
            var annotated=attempt>=2;
            if(attempt==2)
            {
                var ink=(InkCanvas)item.GetType().GetProperty("Markup")!.GetValue(item)!;
                ink.Strokes.Add(new Stroke(new StylusPointCollection{new StylusPoint(20,20),new StylusPoint(180,90)}){DrawingAttributes=new DrawingAttributes{Color=Colors.Blue,Width=5,Height=5}});
                var notes=(List<AiAnnotation>)item.GetType().GetProperty("AnnotationNotes")!.GetValue(item)!;
                notes.Add(new AiAnnotation(.1,.1,.25,.25,"合成移动标记",0,.1,2,[new VideoAnnotationKeyframe(.1,.1,.1,.25,.25),new VideoAnnotationKeyframe(2,.4,.3,.25,.25)]));
            }
            File.AppendAllText(log,$"pin-{attempt}-annotated-{annotated}\n");
            typeof(CaptureOverlayWindow).GetMethod("Pin",Private)!.Invoke(overlay,new object[]{overlay,new RoutedEventArgs()});
            PinnedVideoWindow? pin=null;
            await Until(()=> (pin=Application.Current.Windows.OfType<PinnedVideoWindow>().SingleOrDefault()) is not null,"pin-created");
            pin!.Title="贴视频验收 · 合成画面";
            var player=typeof(PinnedVideoWindow).GetField("_player",Private)!.GetValue(pin)!;
            long Count()=>(long)player.GetType().GetProperty("PresentedFrameCount",Private)!.GetValue(player)!;
            await Until(()=>Count()>=4,"pin-frames");
            File.AppendAllText(log,$"playing-{attempt}\n");
            await Task.Run(()=>{GC.Collect();GC.WaitForPendingFinalizers();GC.Collect();});
            var beforeFrames=Count();
            pin.Width*=1.1;pin.Topmost=false;await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);pin.Topmost=true;
            await Until(()=>Count()>beforeFrames+8,"resized-playing");
            typeof(PinnedVideoWindow).GetMethod("TogglePlayback",Private)!.Invoke(pin,null);
            await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
            typeof(PinnedVideoWindow).GetMethod("TogglePlayback",Private)!.Invoke(pin,null);
            beforeFrames=Count();await Until(()=>Count()>beforeFrames+3,"resumed");
            if(attempt==11)
            {
                overlay.Close();
                await Task.Run(()=>{GC.Collect();GC.WaitForPendingFinalizers();GC.Collect();});
                beforeFrames=Count();await Until(()=>Count()>beforeFrames+8,"pin-survives-overlay-close");
                File.AppendAllText(log,"overlay-closed-pin-still-playing\n");
            }
            pin.Close();await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
            File.AppendAllText(log,$"closed-{attempt}\n");
        }
        var after=SHA256.HashData(await File.ReadAllBytesAsync(source));
        if(!hash.SequenceEqual(after))throw new Exception("Original video changed");
        File.AppendAllText(log,"PASS source unchanged; raw and annotated pin/play/resize/topmost/close\n");
    }
    private static async Task Until(Func<bool> condition,string stage)
    {
        var clock=Stopwatch.StartNew();
        while(!condition()){if(clock.Elapsed>TimeSpan.FromSeconds(60))throw new TimeoutException(stage);await Task.Delay(50);}
    }
}
