// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal enum GradingVerdict { Correct, Incorrect, Blank, Uncertain }
internal sealed record GradingItem(string Question,string Observed,string Expected,GradingVerdict Verdict,string Reason,string Skill,
    double X,double Y,double Width,double Height,decimal? Score=null,decimal? Maximum=null)
{
    public bool Confirmed { get; set; }
}
internal sealed class TeachingPage(string id,string submission,int pageNumber,BitmapSource image,string fingerprint)
{
    internal string Id { get; }=id;
    internal string Submission { get; set; }=submission;
    internal int PageNumber { get; set; }=pageNumber;
    internal BitmapSource Image { get; }=image;
    internal string Fingerprint { get; }=fingerprint;
    internal IReadOnlyList<GradingItem> Items { get; set; }=[];
    internal string Status { get; set; }="";
    internal bool Selected { get; set; }=true;
    internal string Label=>$"{Submission} · {PageNumber}";
}
internal sealed record PracticeItem(string Question,string Answer,string Explanation,string Skill);
internal sealed record CommonLearningIssue(string Skill,IReadOnlyList<string> Evidence,int Submissions);

/// <summary>Explicit, bounded, memory-only collection, shared by successive capture overlays.</summary>
internal sealed class TeachingSession
{
    internal const int PageLimit=16;
    internal const long PixelLimit=32_000_000;
    internal List<TeachingPage> Pages { get; }=[];
    internal IReadOnlyList<PracticeItem> Practice { get; set; }=[];
    internal string Rubric { get; set; }="";
    internal bool PracticeConfirmed { get; set; }
    internal void AddRange(IReadOnlyList<TeachingPage> pages)
    {
        if(Pages.Count+pages.Count>PageLimit||Pages.Concat(pages).Sum(p=>(long)p.Image.PixelWidth*p.Image.PixelHeight)>PixelLimit)
            throw new InvalidOperationException(LocalizationService.T("本次作业最多 16 页、3200 万像素，请移除部分页面。","This collection allows 16 pages and 32 million pixels. Remove some pages first."));
        if(pages.Any(p=>string.IsNullOrWhiteSpace(p.Submission)||p.Submission.Length>80||p.PageNumber<1||p.PageNumber>10000))throw new InvalidDataException("Invalid page identity");
        var combined=Pages.Concat(pages).ToArray();
        if(combined.GroupBy(p=>(p.Submission,p.PageNumber)).Any(g=>g.Count()>1)||combined.GroupBy(p=>(p.Submission,p.Fingerprint)).Any(g=>g.Count()>1))
            throw new InvalidOperationException(LocalizationService.T("同一作答的页码或图片重复，请更改作答代号/页码或移除重复页。","Duplicate page number or image in the same submission. Check the identity and page number."));
        Pages.AddRange(pages);InvalidatePractice();
    }
    internal void InvalidatePractice(){Practice=[];PracticeConfirmed=false;}
    internal void Clear(){Pages.Clear();Rubric="";InvalidatePractice();}
    internal IReadOnlyList<CommonLearningIssue> CommonIssues()=>Pages.SelectMany(p=>p.Items.Where(i=>i.Confirmed&&i.Verdict==GradingVerdict.Incorrect&&!string.IsNullOrWhiteSpace(i.Skill)).Select(i=>new{Page=p,Item=i}))
        .GroupBy(x=>x.Item.Skill.Trim(),StringComparer.OrdinalIgnoreCase)
        .Where(g=>g.Select(x=>x.Page.Submission).Distinct(StringComparer.Ordinal).Count()>=2)
        .Select(g=>new CommonLearningIssue(g.Key,g.Select(x=>$"{x.Page.Label} / {x.Item.Question}: {x.Item.Observed} → {x.Item.Expected}").Distinct().Take(32).ToArray(),g.Select(x=>x.Page.Submission).Distinct().Count())).Take(12).ToArray();
    internal static string VerdictText(GradingVerdict value)=>value switch
    {
        GradingVerdict.Correct=>LocalizationService.T("正确","Correct"),GradingVerdict.Incorrect=>LocalizationService.T("错误","Incorrect"),
        GradingVerdict.Blank=>LocalizationService.T("未作答","Blank"),_=>LocalizationService.T("待核","Uncertain")
    };
    internal static IReadOnlyList<AiAnnotation> Annotations(TeachingPage page,string handle)=>TeachingFeedbackLayout.Annotations(page,handle);
}
