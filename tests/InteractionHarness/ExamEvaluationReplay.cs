// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Collections;
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using TextBox=System.Windows.Controls.TextBox;

// Explicit live evaluation on public exam excerpts and simulated answers only.
// Fixture files and model output stay in ignored .codex-build, never in releases.
internal static class ExamEvaluationReplay
{
    private const BindingFlags Flags=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,AppHost host,CaptureOverlayWindow overlay,string[] args)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"教学能力实测 · 官方试题 / 模拟作答 · 自动结束");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var folder=Path.GetFullPath(".codex-build/teaching-evaluation");
            Directory.CreateDirectory(folder);
            var run=(args.Contains("--api")?"api-":"")+(args.Contains("--handwriting-only")?"handwriting-":"")+(args.Contains("--after")?"after":"baseline");
            var records=new List<object>();string? failure=null;
            File.WriteAllText(Path.Combine(folder,run+"-result.json"),JsonSerializer.Serialize(new{records,failure="Evaluation is running"}));
            using var deadline=new CancellationTokenSource(TimeSpan.FromMinutes(10));
            try
            {
                var runtime=(HermesRuntimeService)typeof(AppHost).GetField("_hermesRuntime",Flags)!.GetValue(host)!;
                string model;
                if(args.Contains("--api"))
                {
                    using var json=JsonDocument.Parse(File.ReadAllText(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json")));
                    var options=new JsonSerializerOptions();options.Converters.Add(new JsonStringEnumConverter());
                    var provider=json.RootElement.GetProperty("Providers").EnumerateArray().First(p=>p.GetProperty("Type").GetString()=="MiniMax"&&p.GetProperty("Model").GetString()=="MiniMax-M3").Deserialize<AiProviderSettings>(options)!;
                    host.Settings.Providers=[provider];host.Settings.DefaultProviderId=provider.Id;host.Settings.ConversationChannelId="api:"+provider.Id;model=provider.Model;
                }
                else
                {
                    var profiles=await runtime.GetAgentOptionsAsync(deadline.Token);
                    if(!profiles.Any(p=>p.Name=="default"))throw new InvalidOperationException("Hermes default profile unavailable");
                    var current=(await runtime.GetModelOptionsAsync("default",false,deadline.Token)).First(m=>m.IsCurrent);
                    host.Settings.HermesEnabled=true;host.Settings.HermesProfile="default";host.Settings.HermesProvider=current.Provider;host.Settings.HermesModel=current.Model;
                    host.Settings.ConversationChannelId="hermes";model=current.Model;
                }
                host.Settings.SaveConversationHistory=false;
                Set("_selectedConversationChannelId",host.Settings.ConversationChannelId);Invoke("RefreshAiFeatureAvailability");
                var root=(Canvas)overlay.FindName("Root");
                var items=new List<object>();
                object Add(string name,int column)
                {
                    using var stream=File.OpenRead(Path.Combine(folder,name+".png"));var decoded=BitmapFrame.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
                    var bgra=new FormatConvertedBitmap(decoded,PixelFormats.Bgra32,null,0);var bytes=new byte[bgra.PixelWidth*bgra.PixelHeight*4];bgra.CopyPixels(bytes,bgra.PixelWidth*4,0);
                    var frame=BitmapSource.Create(bgra.PixelWidth,bgra.PixelHeight,96,96,PixelFormats.Bgra32,null,bytes,bgra.PixelWidth*4);frame.Freeze();Array.Clear(bytes);
                    var item=Invoke("CreateSelection",false)!;var w=Math.Min(Math.Min(root.ActualWidth*.46,820),Math.Max(100,root.ActualHeight-230)*frame.PixelWidth/frame.PixelHeight);var h=w*frame.PixelHeight/frame.PixelWidth;
                    item.GetType().GetField("Bounds")!.SetValue(item,new Rect(root.ActualWidth*.025+column*root.ActualWidth*.50,65,w,h));
                    item.GetType().GetField("CapturedImageOverride")!.SetValue(item,frame);
                    ((IList)Get("_selections")).Add(item);Invoke("UpdateSelection",item);items.Add(item);return item;
                }
                if(args.Contains("--handwriting-only"))
                {
                    Add("handwriting-03",0);
                    await Send("handwriting","请批改这张考试机构公开的真实手写数学答卷中的第1、2题，核对每一步，指出最早出现的错误，特别留意指数和分母正负号；在原作答处直接批注。没有评分细则时不编造具体扣分。用简体中文回答，可以使用 Hermes 既有视觉工具读取本次附件，但不要访问无关文件、联网找答案或生成图片。");
                    return;
                }
                Add("student-A",0);
                if(!args.Contains("--comparison-only"))await Send("single","请批改这张数学试卷，蓝字是学生作答。逐题判断对错、说明错因，在原卷对应作答上直接标注。用简体中文回答；只分析这张试卷，可以使用 Hermes 既有视觉工具读取本次附件，但不要访问无关文件、联网找答案或生成图片。");
                // Send clean image pixels. Hermes intentionally retains its
                // current session, exercising the product's multi-turn path.
                foreach(var item in items)((IList)item.GetType().GetProperty("AnnotationNotes")!.GetValue(item)!).Clear();
                var history=(IList)Get("_history");while(history.Count>1)history.RemoveAt(history.Count-1);
                Add("student-B",1);
                await Send("comparison","这两张图是两位学生对同一份数学试卷的作答，蓝字为各自的答案。请分别批改，找出两人共同的问题，区分个别错误和空白；在原卷相应位置标注，并根据共同错因出4道巩固题，附答案与简短解析。用简体中文回答；可以使用 Hermes 既有视觉工具读取本次附件，但不要访问无关文件、联网找答案或生成图片。");
                var beforeFollowup=JsonSerializer.Serialize(items.Select(Notes));
                await Send("followup","只解释刚才两位学生共同的错因，并再出一道迁移题与答案，不需要改变原卷标注。可以使用 Hermes 既有视觉工具读取本次附件，但不要访问无关文件、联网找答案或生成图片。");
                var preserved=beforeFollowup==JsonSerializer.Serialize(items.Select(Notes));
                records.Add(new{scenario="followup-preserves-annotations",passed=preserved});
                if(!preserved)throw new InvalidOperationException("Follow-up changed grading annotations");
                async Task Send(string scenario,string prompt)
                {
                    var elapsed=System.Diagnostics.Stopwatch.StartNew();
                    File.WriteAllText(Path.Combine(folder,run+"-progress.txt"),scenario);
                    ((TextBox)overlay.FindName("QuickPrompt")).Text=prompt;
                    var timer=new DispatcherTimer{Interval=TimeSpan.FromSeconds(5)};
                    timer.Tick+=(_,_)=>File.WriteAllText(Path.Combine(folder,run+"-progress.txt"),$"{scenario}: {((TextBlock)overlay.FindName("PromptStatus")).Text}; answerChars={((MarkdownAnswerView)overlay.FindName("AnswerText")).PlainText.Length}");
                    timer.Start();
                    try{await ((Task)Invoke("SendAsync",false,null!,null!,false)!).WaitAsync(deadline.Token);}
                    finally{timer.Stop();}
                    var answer=(MarkdownAnswerView)overlay.FindName("AnswerText");
                    var status=((TextBlock)overlay.FindName("PromptStatus")).Text;
                    File.WriteAllText(Path.Combine(folder,$"{run}-{scenario}-answer.md"),answer.PlainText);
                    File.WriteAllText(Path.Combine(folder,$"{run}-{scenario}-markdown.md"),answer.Markdown);
                    var annotations=items.Select((item,i)=>new{index=i,handle=item.GetType().GetProperty("ReferenceHandle")!.GetValue(item),notes=Notes(item).ToArray()}).ToArray();
                    File.WriteAllText(Path.Combine(folder,$"{run}-{scenario}-annotations.json"),JsonSerializer.Serialize(annotations));
                    foreach(var (item,i) in items.Select((item,i)=>(item,i)))
                    {
                        Save((BitmapSource)Invoke("RenderSelectionImage",item,true,true,true)!,Path.Combine(folder,$"{run}-{scenario}-paper-{i}.png"));
                    }
                    records.Add(new{scenario,status,answerLength=answer.PlainText.Length,annotations=annotations.Select(a=>a.notes.Length).ToArray(),model,elapsedSeconds=elapsed.Elapsed.TotalSeconds});
                    if(answer.PlainText.Length==0||!items.Any(i=>Notes(i).Count>0))throw new InvalidOperationException("Live exam request produced no answer or no mapped annotations: "+status);
                    await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);
                }
            }
            catch(Exception ex){failure=ex.GetType().Name+": "+ex.Message;Environment.ExitCode=1;}
            finally
            {
                if(Get("_request") is CancellationTokenSource pending)pending.Cancel();
                File.WriteAllText(Path.Combine(folder,run+"-result.json"),JsonSerializer.Serialize(new{records,failure}));
                File.WriteAllText(Path.Combine(folder,run+"-progress.txt"),failure is null?"Completed":"Failed; see result.json");
                overlay.Close();await ((HermesRuntimeService)typeof(AppHost).GetField("_hermesRuntime",Flags)!.GetValue(host)!).DisposeAsync();app.Shutdown(Environment.ExitCode);
            }
            object Get(string name)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.GetValue(overlay)!;
            void Set(string name,object value)=>typeof(CaptureOverlayWindow).GetField(name,Flags)!.SetValue(overlay,value);
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Flags)!.Invoke(overlay,values);
            static IReadOnlyList<AiAnnotation> Notes(object item)=>(IReadOnlyList<AiAnnotation>)item.GetType().GetProperty("AnnotationNotes")!.GetValue(item)!;
        }));
    }
    private static void Save(BitmapSource image,string path)
    {
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var stream=File.Create(path);encoder.Save(stream);
    }
}
