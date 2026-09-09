// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Views;
using Application=System.Windows.Application;
using Point=System.Windows.Point;

internal static class TeachingWorkflowReplay
{
    private const BindingFlags Flags=BindingFlags.Instance|BindingFlags.NonPublic;
    internal static void Run(Application app,AppHost host,CaptureOverlayWindow overlay,string[] args)
    {
        app.ShutdownMode=ShutdownMode.OnExplicitShutdown;
        Program.MarkReplayWindow(overlay,"教学流程验收 · 公开试题与模拟作答 · 自动结束");
        overlay.Loaded+=(_,_)=>app.Dispatcher.BeginInvoke(new Action(async()=>
        {
            var folder=Path.GetFullPath(".codex-build/teaching-evaluation");Directory.CreateDirectory(folder);
            var run="workflow-"+(args.Contains("--api")?"api":"hermes")+(args.Contains("--handwriting-only")?"-handwriting":"");
            var runtime=(HermesRuntimeService)typeof(AppHost).GetField("_hermesRuntime",Flags)!.GetValue(host)!;
            var results=new List<object>();string? failure=null;
            try
            {
                using var deadline=new CancellationTokenSource(TimeSpan.FromMinutes(14));
                // Validate Windows native PDF rendering against an actual public PDF,
                // including selection of page 11 out of the full 20-page document.
                if(!args.Contains("--skip-pdf"))
                {
                    var pdf=await TeachingImportService.ImportAsync(Path.Combine(folder,"paper-2.pdf"),"PDF", "11",1,deadline.Token);
                    if(pdf.Count!=1||pdf[0].PageNumber!=11||Math.Max(pdf[0].Image.PixelWidth,pdf[0].Image.PixelHeight)>1600)throw new InvalidDataException("PDF import mismatch");
                    results.Add(new{stage="native-pdf",pages=pdf.Count,width=pdf[0].Image.PixelWidth,height=pdf[0].Image.PixelHeight});
                }
                if(args.Contains("--handwriting-only"))host.Teaching.AddRange(await TeachingImportService.ImportAsync(Path.Combine(folder,"real-handwriting-level2.pdf"),"Official handwriting","3",1,deadline.Token));
                else foreach(var name in new[]{"A","B"})host.Teaching.AddRange(await TeachingImportService.ImportAsync(Path.Combine(folder,"student-"+name+".png"),name,"",1,deadline.Token));
                if(args.Contains("--review-saved"))
                {
                    foreach(var page in host.Teaching.Pages)
                    {
                        using var saved=JsonDocument.Parse(File.ReadAllText(Path.Combine(folder,"workflow-api-"+page.Submission+".json")));
                        page.Items=await TeachingGradingService.RefineBoundsAsync(page,saved.RootElement.GetProperty("Items").Deserialize<GradingItem[]>()!,deadline.Token);
                        File.WriteAllText(Path.Combine(folder,"refined-"+page.Submission+".json"),JsonSerializer.Serialize(page.Items));
                    }
                }
                Invoke("ToggleTeaching",overlay,new RoutedEventArgs());Invoke("ShowTeachingPage",host.Teaching.Pages[0]);
                Snapshot("import");
                if(args.Contains("--review-saved"))
                {
                    var page=host.Teaching.Pages[0];var original=page.Items[0];
                    foreach(var previewPage in host.Teaching.Pages){Invoke("ShowTeachingPage",previewPage);Snapshot("refined-"+previewPage.Submission);}
                    Invoke("ShowTeachingPage",page);
                    var preview=typeof(CaptureOverlayWindow).GetField("_teachingPreview",Flags)!.GetValue(overlay)!;var bounds=(Rect)preview.GetType().GetField("Bounds")!.GetValue(preview)!;
                    typeof(CaptureOverlayWindow).GetField("_teachingRepositionQuestion",Flags)!.SetValue(overlay,original.Question);
                    Invoke("TeachingRepositionDown",new Point(bounds.X+bounds.Width*.4,bounds.Y+bounds.Height*.2));Invoke("TeachingRepositionMove",new Point(bounds.X+bounds.Width*.6,bounds.Y+bounds.Height*.3));Invoke("TeachingRepositionUp",new Point(bounds.X+bounds.Width*.6,bounds.Y+bounds.Height*.3));
                    if(Math.Abs(page.Items[0].X-.4)>.0001||Math.Abs(page.Items[0].Height-.1)>.0001)throw new InvalidDataException("Answer-box correction did not update the page");
                    page.Items=page.Items.Select((item,index)=>index==0?original:item).ToArray();Invoke("BuildTeachingPanel");Invoke("RefreshTeachingPreview");
                    var panel=(StackPanel)typeof(CaptureOverlayWindow).GetField("_teachingContent",Flags)!.GetValue(overlay)!;
                    var confirm=Descendants(panel).OfType<System.Windows.Controls.Button>().First(button=>Equals(button.Content,args.Contains("--english")?"Confirm this question":"确认这道题"));
                    confirm.RaiseEvent(new RoutedEventArgs(System.Windows.Controls.Button.ClickEvent));if(!page.Items[0].Confirmed)throw new InvalidDataException("Teacher confirmation was not saved");
                    host.Teaching.Practice=args.Contains("--review-generated-practice")?JsonSerializer.Deserialize<PracticeItem[]>(File.ReadAllText(Path.Combine(folder,"workflow-api-practice.json")))!:[new("2 + 2 = ?","4","2 + 2 = 4","Addition")];host.Teaching.PracticeConfirmed=true;
                    Invoke("ExportTeachingTo",Path.Combine(folder,"workflow-offline-export.zip"));
                    using var zip=System.IO.Compression.ZipFile.OpenRead(Path.Combine(folder,"workflow-offline-export.zip"));
                    if(zip.GetEntry("questions.html") is null||zip.GetEntry("answers.html") is null||zip.GetEntry("annotated/page-01.png") is null)throw new InvalidDataException("Teaching pack missing content");
                    results.Add(new{stage="review-correction-export",passed=true});
                }
                if(!args.Contains("--live")){results.Add(new{stage="offline-ui",passed=true});return;}
                if(args.Contains("--api"))
                {
                    using var json=JsonDocument.Parse(File.ReadAllText(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"MewuAI","settings.json")));
                    var options=new JsonSerializerOptions();options.Converters.Add(new JsonStringEnumConverter());
                    var provider=json.RootElement.GetProperty("Providers").EnumerateArray().First(p=>p.GetProperty("Type").GetString()=="MiniMax"&&p.GetProperty("Model").GetString()=="MiniMax-M3").Deserialize<AiProviderSettings>(options)!;
                    host.Settings.Providers=[provider];host.Settings.DefaultProviderId=provider.Id;host.Settings.ConversationChannelId="api:"+provider.Id;
                }
                else
                {
                    await runtime.GetAgentOptionsAsync(deadline.Token);var current=(await runtime.GetModelOptionsAsync("default",false,deadline.Token)).First(m=>m.IsCurrent);
                    host.Settings.HermesEnabled=true;host.Settings.HermesProfile="default";host.Settings.HermesProvider=current.Provider;host.Settings.HermesModel=current.Model;host.Settings.ConversationChannelId="hermes";
                }
                host.Settings.SaveConversationHistory=false;typeof(CaptureOverlayWindow).GetField("_selectedConversationChannelId",Flags)!.SetValue(overlay,host.Settings.ConversationChannelId);Invoke("RefreshAiFeatureAvailability");
                if(args.Contains("--diagnose-channel"))
                {
                    var recorder=new FixtureResponseRecorder((IAiProvider)Invoke("TeachingProvider")!,folder,run);
                    await new TeachingGradingService().GradeAsync(recorder,host.Teaching.Pages[0],"",null,deadline.Token);return;
                }
                var timer=new DispatcherTimer{Interval=TimeSpan.FromSeconds(5)};
                timer.Tick+=(_,_)=>File.WriteAllText(Path.Combine(folder,run+"-progress.txt"),((TextBlock)overlay.FindName("PromptStatus")).Text);timer.Start();
                try
                {
                    if(args.Contains("--resume-grades"))foreach(var page in host.Teaching.Pages)
                    {
                        using var saved=JsonDocument.Parse(File.ReadAllText(Path.Combine(folder,run+"-"+page.Submission+".json")));
                        page.Items=saved.RootElement.GetProperty("Items").Deserialize<GradingItem[]>()!;
                    }
                    else await ((Task)Invoke("GradeTeachingAsync")!).WaitAsync(deadline.Token);
                }
                finally{timer.Stop();}
                foreach(var page in host.Teaching.Pages)
                {
                    File.WriteAllText(Path.Combine(folder,run+"-"+page.Submission+".json"),JsonSerializer.Serialize(new{page.Submission,page.PageNumber,page.Items}));
                    results.Add(new{stage="grade",page.Submission,count=page.Items.Count,uncertain=page.Items.Count(i=>i.Verdict==GradingVerdict.Uncertain)});
                }
                Snapshot("graded");
                if(host.Teaching.Pages.Any(p=>p.Items.Count==0))throw new InvalidDataException("One or more pages did not finish: "+((TextBlock)overlay.FindName("PromptStatus")).Text);
                if(args.Contains("--handwriting-only"))return;
                var expected=new[]{new[]{GradingVerdict.Correct,GradingVerdict.Incorrect,GradingVerdict.Correct,GradingVerdict.Incorrect,GradingVerdict.Correct,GradingVerdict.Incorrect},new[]{GradingVerdict.Incorrect,GradingVerdict.Correct,GradingVerdict.Incorrect,GradingVerdict.Incorrect,GradingVerdict.Blank,GradingVerdict.Incorrect}};
                var missed=0;var correct=0;var uncertain=0;
                for(var n=0;n<host.Teaching.Pages.Count;n++)
                {
                    var page=host.Teaching.Pages[n];if(page.Items.Count!=6)throw new InvalidDataException("Expected exactly six questions");
                    for(var q=24;q<=29;q++)
                    {
                        var row=page.Items.Single(i=>i.Question.Trim().TrimEnd('.')==q.ToString());
                        if(row.Verdict==GradingVerdict.Uncertain)uncertain++;else if(row.Verdict==expected[n][q-24])correct++;else missed++;
                    }
                }
                results.Add(new{stage="ground-truth",correct,uncertain,missed});
                if(missed>0)throw new InvalidDataException("Model produced confidently incorrect verdicts");
                // Scripted teacher review of known fixture truth; never infer
                // this confirmation from the model agreeing with itself.
                foreach(var page in host.Teaching.Pages)
                {
                    page.Items=page.Items.Select(row=>row with{Skill=row.Question=="27"?"代数式去括号与合并同类项":row.Question=="29"?"完全平方公式":row.Skill,Confirmed=row.Verdict!=GradingVerdict.Uncertain}).ToArray();
                }
                await ((Task)Invoke("PracticeTeachingAsync")!).WaitAsync(deadline.Token);
                if(host.Teaching.Practice.Count!=4)throw new InvalidDataException("Practice generation failed: "+((TextBlock)overlay.FindName("PromptStatus")).Text);
                File.WriteAllText(Path.Combine(folder,run+"-practice.json"),JsonSerializer.Serialize(host.Teaching.Practice));
                results.Add(new{stage="practice",count=host.Teaching.Practice.Count,common=host.Teaching.CommonIssues().Count});Snapshot("practice");
            }
            catch(Exception ex){failure=ex.GetBaseException().Message;Environment.ExitCode=1;}
            finally
            {
                File.WriteAllText(Path.Combine(folder,run+"-result.json"),JsonSerializer.Serialize(new{results,failure}));
                overlay.Close();await runtime.DisposeAsync();app.Shutdown(Environment.ExitCode);
            }
            object? Invoke(string name,params object[] values)=>typeof(CaptureOverlayWindow).GetMethod(name,Flags)!.Invoke(overlay,values);
            void Snapshot(string stage)
            {
                overlay.UpdateLayout();var image=new RenderTargetBitmap((int)overlay.ActualWidth,(int)overlay.ActualHeight,96,96,PixelFormats.Pbgra32);image.Render(overlay);
                var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(image));using var output=File.Create(Path.Combine(folder,run+"-"+stage+".png"));encoder.Save(output);
            }
        }));
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject parent)
    {
        foreach(var child in LogicalTreeHelper.GetChildren(parent).OfType<DependencyObject>()){yield return child;foreach(var descendant in Descendants(child))yield return descendant;}
    }
    private sealed class FixtureResponseRecorder(IAiProvider inner,string folder,string run):IAiProvider
    {
        private int _count;
        public string Id=>inner.Id;public AiProviderCapabilities Capabilities=>inner.Capabilities;
        public Task<bool> TestConnectionAsync(CancellationToken token)=>inner.TestConnectionAsync(token);
        public async Task<AiResult> SendAsync(AiRequest request,CancellationToken token)
        {
            var result=await inner.SendAsync(request,token);File.WriteAllText(Path.Combine(folder,run+"-diagnostic-"+(++_count)+".txt"),result.Answer);return result;
        }
    }
}
