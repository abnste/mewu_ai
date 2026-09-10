// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Microsoft.Win32;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;
using mewu_ai_Assistant.Recording;
using mewu_ai_Assistant.Services;
using Button=System.Windows.Controls.Button;
using TextBox=System.Windows.Controls.TextBox;

namespace mewu_ai_Assistant.Views;

public partial class CaptureOverlayWindow
{
    private Border? _teachingPanel;
    private StackPanel? _teachingContent;
    private CancellationTokenSource? _teachingRequest;
    private SelectionItem? _teachingPreview;
    private TeachingPage? _teachingPage;
    private TextBox? _teachingIdentity,_teachingPageNumber,_teachingPdfRange,_teachingRubric;
    private TextBlock? _teachingStatus;
    private string? _teachingRepositionQuestion;
    private System.Windows.Point? _teachingRepositionStart;
    private Border? _teachingRepositionOutline;
    private TeachingSession Teaching=>_host.Teaching;
    private static string L(string zh,string en)=>LocalizationService.T(zh,en);

    private void ToggleTeaching(object sender,RoutedEventArgs e)
    {
        if(_teachingPanel is {Visibility:Visibility.Visible}){_teachingPanel.Visibility=Visibility.Collapsed;SetPromptBarHidden(false);return;}
        if(_request is not null||_overlayRequest is not null||_recordingMode||_drawingMode||_longCaptureMode)return;
        if(_teachingPanel is null)
        {
            _teachingContent=new StackPanel();
            _teachingPanel=new Border{Width=410,Background=Brushes.White,BorderBrush=new SolidColorBrush(Color.FromRgb(215,225,239)),BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(18),Padding=new Thickness(16),
                Child=new ScrollViewer{VerticalScrollBarVisibility=ScrollBarVisibility.Auto,Content=_teachingContent}};
            Root.Children.Add(_teachingPanel);Panel.SetZIndex(_teachingPanel,600);
            LocalizationService.SetExcludeFromLocalization(_teachingPanel,true);
        }
        _teachingPanel.MaxHeight=Math.Max(220,Root.ActualHeight-180);Canvas.SetLeft(_teachingPanel,Math.Max(12,Root.ActualWidth-_teachingPanel.Width-20));Canvas.SetTop(_teachingPanel,20);
        _teachingPanel.Visibility=Visibility.Visible;BuildTeachingPanel();Toolbar.Visibility=Visibility.Collapsed;
    }
    private bool IsTeachingControl(DependencyObject? source)=>_teachingPanel is {Visibility:Visibility.Visible}&&IsInside(source,_teachingPanel);
    private void TeachingMessage(string text){if(_closed)return;if(_teachingStatus is not null)_teachingStatus.Text=text;PromptStatus.Text=text;}
    private void BuildTeachingPanel()
    {
        if(_teachingContent is null)return;
        var identity=_teachingIdentity?.Text??"A";var number=_teachingPageNumber?.Text??"1";var range=_teachingPdfRange?.Text??"";
        var panel=_teachingContent;panel.Children.Clear();
        AddText(panel,L("试卷与作业","Papers and assignments"),19,true);
        AddText(panel,L("页面仅保留在本次应用会话；点击批改才发送。可关闭截图、翻页，再次截图后继续收集。","Pages stay in this app session. Only Grade sends them. Close capture, turn the page, then capture again to collect more."));
        var inputs=new Grid();inputs.ColumnDefinitions.Add(new());inputs.ColumnDefinitions.Add(new(){Width=new GridLength(75)});
        _teachingIdentity=new TextBox{Text=identity,MaxLength=80,ToolTip=L("作答代号：同一学生的多页使用相同代号","Submission ID: use the same ID for all pages from one student"),Margin=new Thickness(0,5,8,5)};
        _teachingPageNumber=new TextBox{Text=number,MaxLength=5,ToolTip=L("起始页码","First page number"),Margin=new Thickness(0,5,0,5)};
        AddText(panel,L("作答代号 / 起始页码","Submission ID / First page"));inputs.Children.Add(_teachingIdentity);inputs.Children.Add(_teachingPageNumber);Grid.SetColumn(_teachingPageNumber,1);panel.Children.Add(inputs);
        _teachingPdfRange=new TextBox{Text=range,MaxLength=160,ToolTip=L("PDF 页码，例如 3,5-7；留空取前16页","PDF pages, e.g. 3,5-7; blank selects the first 16"),Margin=new Thickness(0,4,0,4)};
        AddText(panel,L("PDF 选页（例：3,5-7；留空取前16页）","PDF pages (e.g. 3,5-7; blank: first 16)"));panel.Children.Add(_teachingPdfRange);
        var actions=new WrapPanel();panel.Children.Add(actions);
        AddButton(actions,L("导入 PDF / 图片","Import PDF / images"),async()=>await ImportTeachingAsync());
        AddButton(actions,L("收集截图","Collect regions"),CollectTeachingSelections);
        AddButton(actions,L("继续截图","Capture next"),()=>{if(_teachingRequest is null)Close();});
        _teachingStatus=AddText(panel,L($"已收集 {Teaching.Pages.Count}/16 页；每次最多批改4页。","Collected "+Teaching.Pages.Count+"/16 pages; grade up to 4 at a time."));
        foreach(var page in Teaching.Pages.ToArray())
        {
            var row=new WrapPanel{Margin=new Thickness(0,3,0,3)};var check=new CheckBox{IsChecked=page.Selected,VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(0,0,5,0),IsEnabled=_teachingRequest is null};
            check.Checked+=(_,_)=>page.Selected=true;check.Unchecked+=(_,_)=>page.Selected=false;row.Children.Add(check);
            AddButton(row,page.Label,()=>ShowTeachingPage(page));
            AddButton(row,L("改代号/页码","Set identity"),()=>ChangeTeachingIdentity(page));
            AddButton(row,"×",()=>{Teaching.Pages.Remove(page);Teaching.InvalidatePractice();if(ReferenceEquals(_teachingPage,page))ClearTeachingPreview();BuildTeachingPanel();});
            panel.Children.Add(row);if(page.Status.Length>0)AddText(panel,page.Status);
        }
        AddText(panel,L("评分细则（可选；更改后需重新批改）","Scoring rubric (optional; changes require regrading)"));
        _teachingRubric=new TextBox{Text=Teaching.Rubric,MaxLength=8000,AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,Height=62,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,IsEnabled=_teachingRequest is null};
        _teachingRubric.TextChanged+=(_,_)=>{if(Teaching.Rubric==_teachingRubric.Text)return;Teaching.Rubric=_teachingRubric.Text;foreach(var p in Teaching.Pages){p.Items=[];p.Status=L("细则已变更，需重新批改","Rubric changed; regrade required");}Teaching.InvalidatePractice();RefreshTeachingPreview();};panel.Children.Add(_teachingRubric);
        var aiActions=new WrapPanel();panel.Children.Add(aiActions);
        AddButton(aiActions,L("批改勾选页","Grade selected"),async()=>await GradeTeachingAsync());
        AddButton(aiActions,L("共性分析与出题","Shared errors & practice"),async()=>await PracticeTeachingAsync());
        AddButton(aiActions,L("导出教学包","Export teaching pack"),ExportTeaching);
        var cancel=new Button{Content=L("停止","Stop"),Margin=new Thickness(3),IsEnabled=_teachingRequest is not null};cancel.Click+=(_,_)=>_teachingRequest?.Cancel();aiActions.Children.Add(cancel);
        if(_teachingPage is { } current&&Teaching.Pages.Contains(current))BuildTeachingReview(panel,current);
        if(Teaching.Practice.Count>0)
        {
            CheckBox? practiceConfirmation=null;
            AddText(panel,L("巩固练习 · AI 复核草稿，需老师核对","Practice · AI-reviewed draft; teacher review required"),15,true);
            foreach(var (item,index) in Teaching.Practice.Select((item,index)=>(item,index)))
            {
                AddText(panel,$"{index+1}. {item.Skill}",14,true);
                var question=new TextBox{Text=item.Question,MaxLength=2000,AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,MinHeight=48,MaxHeight=110,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,IsEnabled=_teachingRequest is null};
                var answer=new TextBox{Text=item.Answer,MaxLength=1000,TextWrapping=TextWrapping.Wrap,IsEnabled=_teachingRequest is null};
                var explanation=new TextBox{Text=item.Explanation,MaxLength=2000,AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,MinHeight=48,MaxHeight=110,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,IsEnabled=_teachingRequest is null};
                panel.Children.Add(question);panel.Children.Add(answer);panel.Children.Add(explanation);
                void UpdateDraft(object sender,TextChangedEventArgs args)
                {
                    Teaching.Practice=Teaching.Practice.Select((p,n)=>n==index?p with{Question=question.Text.Trim(),Answer=answer.Text.Trim(),Explanation=explanation.Text.Trim()}:p).ToArray();
                    Teaching.PracticeConfirmed=false;if(practiceConfirmation is not null)practiceConfirmation.IsChecked=false;
                }
                question.TextChanged+=UpdateDraft;answer.TextChanged+=UpdateDraft;explanation.TextChanged+=UpdateDraft;
                AddButton(panel,L("保存这道练习的修改","Save exercise changes"),()=>
                {
                    if(string.IsNullOrWhiteSpace(question.Text)||string.IsNullOrWhiteSpace(answer.Text)||string.IsNullOrWhiteSpace(explanation.Text))throw new InvalidOperationException(L("题目、答案和解析不能为空。","Question, answer and explanation cannot be empty."));
                    Teaching.Practice=Teaching.Practice.Select((p,n)=>n==index?p with{Question=question.Text.Trim(),Answer=answer.Text.Trim(),Explanation=explanation.Text.Trim()}:p).ToArray();Teaching.PracticeConfirmed=false;BuildTeachingPanel();
                });
            }
            var confirm=new CheckBox{Content=L("我已核对练习与答案","I reviewed the questions and answers"),IsChecked=Teaching.PracticeConfirmed,Margin=new Thickness(0,8,0,8),IsEnabled=_teachingRequest is null};practiceConfirmation=confirm;confirm.Checked+=(_,_)=>Teaching.PracticeConfirmed=true;confirm.Unchecked+=(_,_)=>Teaching.PracticeConfirmed=false;panel.Children.Add(confirm);
        }
        var foot=new WrapPanel();panel.Children.Add(foot);AddButton(foot,L("清空本次作业","Clear collection"),()=>{Teaching.Clear();ClearTeachingPreview();BuildTeachingPanel();});
        AddButton(foot,L("收起","Close panel"),()=>{_teachingPanel!.Visibility=Visibility.Collapsed;SetPromptBarHidden(false);});
    }
    private Button AddButton(Panel panel,string text,Action action)
    {
        var button=new Button{Content=text,Margin=new Thickness(2),Padding=new Thickness(8,5,8,5),IsEnabled=_teachingRequest is null};
        button.Click+=(_,_)=>{if(_teachingRequest is not null)return;try{action();}catch(Exception ex){TeachingMessage(ex.Message);}};panel.Children.Add(button);return button;
    }
    private static TextBlock AddText(Panel panel,string text,double size=12,bool bold=false)
    {
        var item=new TextBlock{Text=text,TextWrapping=TextWrapping.Wrap,FontSize=size,FontWeight=bold?FontWeights.SemiBold:FontWeights.Normal,Foreground=new SolidColorBrush(Color.FromRgb(49,65,87)),Margin=new Thickness(0,5,0,5)};panel.Children.Add(item);return item;
    }
    private (string Identity,int Number) TeachingIdentity()
    {
        var identity=_teachingIdentity?.Text.Trim()??"";
        if(identity.Length==0||!int.TryParse(_teachingPageNumber?.Text,out var number)||number is <1 or >10000)throw new InvalidOperationException(L("请填写作答代号和有效页码。","Enter a submission ID and a valid page number."));return(identity,number);
    }
    private void ChangeTeachingIdentity(TeachingPage page)
    {
        var (identity,number)=TeachingIdentity();
        if(Teaching.Pages.Any(p=>!ReferenceEquals(p,page)&&p.Submission==identity&&(p.PageNumber==number||p.Fingerprint==page.Fingerprint)))throw new InvalidOperationException(L("该作答中已有此页或相同图片。","This submission already has this page or image."));
        page.Submission=identity;page.PageNumber=number;foreach(var i in page.Items)i.Confirmed=false;Teaching.InvalidatePractice();BuildTeachingPanel();RefreshTeachingPreview();
    }
    private async Task ImportTeachingAsync()
    {
        try
        {
            var (identity,number)=TeachingIdentity();var range=_teachingPdfRange!.Text;
            var dialog=new OpenFileDialog{Multiselect=true,Filter=L("试卷 PDF / 图片|*.pdf;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp","PDF / images|*.pdf;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp")};
            if(ShowSystemFileDialog(dialog)!=true)return;
            await RunTeachingAsync(async token=>
            {
                var pending=new List<TeachingPage>();
                foreach(var path in dialog.FileNames)
                {
                    token.ThrowIfCancellationRequested();TeachingMessage(L("正在本地读取页面…","Reading pages locally…"));
                    var pages=await TeachingImportService.ImportAsync(path,identity,range,number,token);pending.AddRange(pages);number=pages.Max(p=>p.PageNumber)+1;
                    if(pending.Count+Teaching.Pages.Count>16)throw new InvalidOperationException(L("最多收集16页，请缩小选页范围。","Select at most 16 pages."));
                }
                token.ThrowIfCancellationRequested();if(_closed)return;Teaching.AddRange(pending);_teachingPageNumber!.Text=number.ToString();if(pending.Count>0)ShowTeachingPage(pending[0]);
            });
        }
        catch(Exception ex){TeachingMessage(ex.Message);}
    }
    private void CollectTeachingSelections()
    {
        var (identity,number)=TeachingIdentity();var items=_selections.Where(p=>!p.IsImplicit&&p.VideoPath is null&&!ReferenceEquals(p,_teachingPreview)).ToArray();
        if(items.Length==0)throw new InvalidOperationException(L("请先框选要收集的作答区域。","Select an answer region first."));
        var pages=items.Select((item,index)=>TeachingImportService.Create(RenderSelectionImage(item,false,false,false),identity,number+index)).ToArray();
        Teaching.AddRange(pages);_teachingPageNumber!.Text=(number+pages.Length).ToString();BuildTeachingPanel();
    }
    private void ClearTeachingPreview()
    {
        CancelTeachingReposition();
        if(_teachingPreview is { } old){_selections.Remove(old);_references.Remove(old);SelectionLayer.Children.Remove(old.Host);ReleaseSelectionResources(old);_activeIndex=_selections.Count-1;}
        _teachingPreview=null;_teachingPage=null;UpdateReferenceChips();QueueSelectionResourceCleanup();
    }
    private void ShowTeachingPage(TeachingPage page)
    {
        if(_closed)return;ClearTeachingPreview();_teachingPage=page;
        var item=CreateSelection(false);item.CapturedImageOverride=page.Image;
        var width=Math.Max(100,Root.ActualWidth-460);var height=Math.Max(100,Root.ActualHeight-230);var scale=Math.Min(width/page.Image.PixelWidth,height/page.Image.PixelHeight);
        item.Bounds=new Rect(20,25,page.Image.PixelWidth*scale,page.Image.PixelHeight*scale);
        _teachingPreview=item;_selections.Add(item);_references.Add(item);_activeIndex=_selections.Count-1;RefreshSelectionNumbers();RefreshTeachingPreview();UpdateSelection(item);BuildTeachingPanel();
    }
    private void RefreshTeachingPreview()
    {
        if(_teachingPreview is not { } item||_teachingPage is not { } page)return;
        item.AnnotationNotes.Clear();item.AnnotationNotes.AddRange(TeachingSession.Annotations(page,item.ReferenceHandle));RenderAnnotationsForItem(item);UpdateReferenceChips();
    }
    private IAiProvider TeachingProvider()
    {
        var channel=_conversationChannels.FirstOrDefault(c=>c.Id==_selectedConversationChannelId)??_conversationChannels.FirstOrDefault();
        if(channel is null)throw new InvalidOperationException(L("请先选择可用 AI 渠道。","Select an available AI channel first."));
        return _host.CreateConversationProvider(HermesConversationKind.Screen,channel.Id,out var error)??throw new InvalidOperationException(error);
    }
    private async Task RunTeachingAsync(Func<CancellationToken,Task> action)
    {
        if(_teachingRequest is not null||RejectIfOverlayOperationBusy())return;
        using var request=new CancellationTokenSource(TimeSpan.FromMinutes(15));_teachingRequest=request;BuildTeachingPanel();
        try{await action(request.Token);request.Token.ThrowIfCancellationRequested();if(!_closed){BuildTeachingPanel();TeachingMessage(L("操作完成，请核对结果。","Completed. Review the results."));}}
        catch(OperationCanceledException){TeachingMessage(request.IsCancellationRequested?L("已停止，保留此前已完成的页面。","Stopped. Previously completed pages are retained."):L("当前渠道等待超时，保留此前已完成的页面；可单独重试未完成页。","The channel timed out. Completed pages are retained; retry the unfinished page."));}
        catch(Exception ex){TeachingMessage(ex is System.Text.Json.JsonException or KeyNotFoundException or FormatException
            ?L("当前渠道未返回可用的批改结果。请检查视觉能力或额度，或切换渠道后重试未完成页。","The channel returned no usable grading result. Check its vision support or quota, or switch channels and retry the unfinished page.")
            :L("本次操作未完成：","Operation incomplete: ")+ex.Message);}
        finally{if(ReferenceEquals(_teachingRequest,request)){_teachingRequest=null;if(!_closed){var message=_teachingStatus?.Text;BuildTeachingPanel();if(message is not null)TeachingMessage(message);}}}
    }
    private async Task GradeTeachingAsync()
    {
        try
        {
            var selected=Teaching.Pages.Where(p=>p.Selected).ToArray();if(selected.Length is <1 or >4)throw new InvalidOperationException(L("每次勾选1至4页；其他页可在下一批继续。","Select 1–4 pages per batch."));
            var provider=TeachingProvider();var rubric=Teaching.Rubric;
            await RunTeachingAsync(async token=>
            {
                for(var n=0;n<selected.Length;n++)
                {
                    var page=selected[n];var index=n;
                    var progress=new Progress<string>(text=>{if(!_closed&&!token.IsCancellationRequested&&_teachingRequest is not null)TeachingMessage($"{index+1}/{selected.Length} · {page.Label} · {text}");});
                    page.Status=L("正在批改…","Grading…");
                    try
                    {
                        var result=await new TeachingGradingService().GradeAsync(provider,page,rubric,progress,token,HandleOverlayInteractionAsync);
                        token.ThrowIfCancellationRequested();if(_closed)return;page.Items=result;page.Selected=false;page.Status=L("两轮批改完成 · 待老师核对","Two passes complete · Review required");Teaching.InvalidatePractice();ShowTeachingPage(page);
                    }
                    catch{page.Status=page.Items.Count>0?L("本轮未完成，保留上次结果","Incomplete; previous results retained"):L("未完成，可单独重试此页","Incomplete; retry this page");throw;}
                }
            });
        }
        catch(Exception ex){TeachingMessage(ex.Message);}
    }
    private void BuildTeachingReview(Panel panel,TeachingPage page)
    {
        AddText(panel,L("逐题核对 · ","Review · ")+page.Label,15,true);
        var scored=page.Items.Where(i=>i.Confirmed&&i.Verdict!=GradingVerdict.Uncertain&&i.Score.HasValue&&i.Maximum.HasValue).ToArray();
        AddText(panel,L($"已核对 {page.Items.Count(i=>i.Confirmed)}/{page.Items.Count} 题",$"Reviewed {page.Items.Count(i=>i.Confirmed)}/{page.Items.Count}"));
        if(scored.Length>0)AddText(panel,L($"已核对且有细则的题：{scored.Sum(i=>i.Score)}/{scored.Sum(i=>i.Maximum)} 分（{scored.Length}题）",$"Rubric-scored subset: {scored.Sum(i=>i.Score)}/{scored.Sum(i=>i.Maximum)} ({scored.Length} questions)"));
        foreach(var original in page.Items)
        {
            var card=new StackPanel();panel.Children.Add(new Border{Child=card,BorderBrush=new SolidColorBrush(Color.FromRgb(224,231,241)),BorderThickness=new Thickness(0,0,0,1),Padding=new Thickness(0,6,0,8)});
            AddText(card,original.Question+" · "+TeachingSession.VerdictText(original.Verdict),14,true);
            AddButton(card,L("重新框出作答位置","Adjust answer box"),()=>{CancelTeachingReposition();_teachingRepositionQuestion=original.Question;TeachingMessage(L("在左侧原卷上拖动，重新框出这道题的作答。","Drag on the original page to outline this answer."));});
            AddText(card,L("识读 / 正确答案","Read answer / Expected answer"));
            var observed=CreateTeachingAnswerEditor(original.Observed);var expected=CreateTeachingAnswerEditor(original.Expected);card.Children.Add(observed);card.Children.Add(expected);
            AddText(card,L("批改说明","Review explanation"));var reason=CreateTeachingAnswerEditor(original.Reason,800,false);card.Children.Add(reason);
            var verdict=new ComboBox{ItemsSource=Enum.GetValues<GradingVerdict>().Select(v=>new KeyValuePair<GradingVerdict,string>(v,TeachingSession.VerdictText(v))),DisplayMemberPath="Value",SelectedValuePath="Key",SelectedValue=original.Verdict,Margin=new Thickness(0,5,0,5)};card.Children.Add(verdict);
            var skill=new TextBox{Text=original.Skill,MaxLength=80,ToolTip=L("统一知识点名称用于共性统计","Use consistent skill names for shared-error grouping")};card.Children.Add(skill);
            var score=new TextBox{Text=original.Score?.ToString(System.Globalization.CultureInfo.InvariantCulture)??"",ToolTip=L("得分（有评分细则时）","Score (requires rubric)")};
            var maximum=new TextBox{Text=original.Maximum?.ToString(System.Globalization.CultureInfo.InvariantCulture)??"",ToolTip=L("该题满分","Maximum score")};
            if(Teaching.Rubric.Length>0){AddText(card,L("得分 / 满分（可留空）","Score / Maximum (optional)"));card.Children.Add(score);card.Children.Add(maximum);}
            AddButton(card,original.Confirmed?L("更新核对结果","Update review"):L("确认这道题","Confirm this question"),()=>
            {
                decimal? s=null,m=null;if(Teaching.Rubric.Length>0&&(score.Text.Length>0||maximum.Text.Length>0))
                {
                    if(!decimal.TryParse(score.Text,System.Globalization.NumberStyles.Number,System.Globalization.CultureInfo.InvariantCulture,out var sv)||!decimal.TryParse(maximum.Text,System.Globalization.NumberStyles.Number,System.Globalization.CultureInfo.InvariantCulture,out var mv)||sv<0||mv<=0||mv>1000||sv>mv)throw new InvalidOperationException(L("分数必须介于0和该题满分之间。","The score must be between zero and the maximum."));s=sv;m=mv;
                }
                var decision=(GradingVerdict)verdict.SelectedValue;
                if(decision==GradingVerdict.Uncertain){s=null;m=null;}
                if(decision==GradingVerdict.Blank&&observed.Text.Trim().Length>0)throw new InvalidOperationException(L("未作答题的识读内容应为空。","A blank response must have no recognized answer."));
                var revised=original with{Observed=observed.Text.Trim(),Expected=expected.Text.Trim(),Reason=reason.Text.Trim(),Verdict=decision,Skill=skill.Text.Trim(),Score=s,Maximum=m,Confirmed=true};
                page.Items=page.Items.Select(i=>ReferenceEquals(i,original)?revised:i).ToArray();Teaching.InvalidatePractice();RefreshTeachingPreview();BuildTeachingPanel();
            });
        }
    }
    private static TextBox CreateTeachingAnswerEditor(string text,int limit=600,bool calculation=true)=>new()
    {
        Text=calculation?TeachingCalculationLayout.Format(text):text,MaxLength=limit,AcceptsReturn=true,TextWrapping=TextWrapping.Wrap,
        MinLines=1,MaxLines=10,MaxHeight=240,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,
        HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,VerticalContentAlignment=VerticalAlignment.Top
    };
    private bool TeachingRepositionDown(System.Windows.Point point)
    {
        if(_teachingRepositionQuestion is null||_teachingPreview is null)return false;
        if(!_teachingPreview.Bounds.Contains(point))return true;
        _teachingRepositionStart=point;Root.CaptureMouse();return true;
    }
    private bool TeachingRepositionMove(System.Windows.Point point)
    {
        if(_teachingRepositionQuestion is null)return false;
        if(_teachingRepositionStart is not { } start||_teachingPreview is null)return true;
        var bounds=_teachingPreview.Bounds;point=new System.Windows.Point(Math.Clamp(point.X,bounds.Left,bounds.Right),Math.Clamp(point.Y,bounds.Top,bounds.Bottom));var rect=new Rect(start,point);
        if(_teachingRepositionOutline is null){_teachingRepositionOutline=new Border{BorderBrush=Brushes.OrangeRed,BorderThickness=new Thickness(2),IsHitTestVisible=false};Root.Children.Add(_teachingRepositionOutline);Panel.SetZIndex(_teachingRepositionOutline,599);}
        Canvas.SetLeft(_teachingRepositionOutline,rect.X);Canvas.SetTop(_teachingRepositionOutline,rect.Y);_teachingRepositionOutline.Width=rect.Width;_teachingRepositionOutline.Height=rect.Height;return true;
    }
    private bool TeachingRepositionUp(System.Windows.Point point)
    {
        if(_teachingRepositionQuestion is null)return false;
        if(_teachingRepositionStart is { } start&&_teachingPreview is { } preview&&_teachingPage is { } page)
        {
            var bounds=preview.Bounds;point=new System.Windows.Point(Math.Clamp(point.X,bounds.Left,bounds.Right),Math.Clamp(point.Y,bounds.Top,bounds.Bottom));var rect=new Rect(start,point);
            if(rect.Width>=3&&rect.Height>=3){page.Items=page.Items.Select(i=>i.Question==_teachingRepositionQuestion?i with{X=(rect.X-bounds.X)/bounds.Width,Y=(rect.Y-bounds.Y)/bounds.Height,Width=rect.Width/bounds.Width,Height=rect.Height/bounds.Height}:i).ToArray();RefreshTeachingPreview();}
        }
        CancelTeachingReposition();BuildTeachingPanel();return true;
    }
    private void CancelTeachingReposition()
    {
        _teachingRepositionQuestion=null;_teachingRepositionStart=null;
        if(_teachingRepositionOutline is not null){Root.Children.Remove(_teachingRepositionOutline);_teachingRepositionOutline=null;}
        if(Root.IsMouseCaptured)Root.ReleaseMouseCapture();
    }
    private async Task PracticeTeachingAsync()
    {
        try
        {
            var issues=Teaching.CommonIssues();var provider=TeachingProvider();
            if(issues.Count==0)throw new InvalidOperationException(L("请先逐题确认至少两份作答的共同错误，并统一知识点名称。","Confirm shared errors in at least two submissions using consistent skill names."));
            await RunTeachingAsync(async token=>
            {
                TeachingMessage(L("正在依据已核对错题出题并独立验算…","Generating and independently checking exercises from reviewed errors…"));
                var practice=await TeachingExportService.CreatePracticeAsync(provider,issues,token);token.ThrowIfCancellationRequested();if(_closed)return;Teaching.Practice=practice;Teaching.PracticeConfirmed=false;
                ShowAnswer();RefreshAnswer(string.Join("\n\n",issues.Select(i=>$"**{i.Skill}** · {i.Submissions} "+L("份作答","submissions")+"\n\n"+string.Join("\n",i.Evidence.Select(e=>"- "+e)))));SetPromptBarHidden(false);
            });
        }
        catch(Exception ex){TeachingMessage(ex.Message);}
    }
    private void ExportTeaching()
    {
        if(Teaching.Pages.Count==0)throw new InvalidOperationException(L("请先收集页面。","Collect pages first."));
        if(Teaching.Practice.Count>0&&!Teaching.PracticeConfirmed)throw new InvalidOperationException(L("请先核对练习与答案并勾选确认。","Review the exercises and check the confirmation box first."));
        if(Teaching.Practice.Any(i=>string.IsNullOrWhiteSpace(i.Question)||string.IsNullOrWhiteSpace(i.Answer)||string.IsNullOrWhiteSpace(i.Explanation)))throw new InvalidOperationException(L("题目、答案和解析不能为空。","Question, answer and explanation cannot be empty."));
        var dialog=new SaveFileDialog{Filter="ZIP|*.zip",DefaultExt=".zip",FileName="MewuAI-Teaching-"+DateTime.Now.ToString("yyyyMMdd-HHmmss")+".zip"};if(ShowSystemFileDialog(dialog)!=true)return;
        ExportTeachingTo(dialog.FileName);
    }
    private void ExportTeachingTo(string path)
    {
        var images=new List<(string Name,byte[] Data)>();
        try
        {
            foreach(var (page,n) in Teaching.Pages.Select((page,n)=>(page,n)))
            {
                var notes=TeachingSession.Annotations(page,page.Id);var ai=AnnotationOverlayRenderer.RenderAiOverlay(page.Image.PixelWidth,page.Image.PixelHeight,notes,null,null);
                var bitmap=AnnotationOverlayRenderer.Composite(page.Image,ai);var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var memory=new MemoryStream();
                try{encoder.Save(memory);images.Add(($"annotated/page-{n+1:D2}.png",memory.ToArray()));}finally{if(memory.TryGetBuffer(out var buffer))Array.Clear(buffer.Array!,buffer.Offset,buffer.Count);}
            }
            TeachingExportService.Export(path,Teaching,images);TeachingMessage(L("已导出标注页、逐题表格及分开的练习/答案，可在浏览器打印。","Exported annotated pages, review table, and separate printable questions/answers."));
        }
        finally{foreach(var image in images)Array.Clear(image.Data);}
    }
}
