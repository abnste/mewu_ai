// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.IO.Compression;
using System.Net;
using System.Text;
using mewu_ai_Assistant.AI;

namespace mewu_ai_Assistant.Services;

internal static class TeachingExportService
{
    internal static async Task<IReadOnlyList<PracticeItem>> CreatePracticeAsync(IAiProvider provider,IReadOnlyList<CommonLearningIssue> issues,CancellationToken token)
    {
        if(issues.Count==0)throw new InvalidOperationException(LocalizationService.T("请先核对至少两份不同作答的共同错题。","First review shared errors in at least two different submissions."));
        using var deadline=CancellationTokenSource.CreateLinkedTokenSource(token);deadline.CancelAfter(TimeSpan.FromMinutes(4));token=deadline.Token;
        var skills=issues.Select(i=>i.Skill).ToHashSet(StringComparer.Ordinal);
        var prompt="根据以下已由老师核对的共性错题证据，出4道由基础纠正到迁移的全新巩固题，不直接重复原题或重复本组其他题。只考查证据中的知识点，skill严格原样使用证据中的名称，不引入其他公式或新知识点。不要引用未提供的学生资料。每题独立可作答，明确单位/条件，涉及长度和面积时明确变量范围并保证各边为正；若边长写为(2x+3)厘米，则x是纯数，不能写成x=1厘米。独立验算答案。每题题干不超过120字，答案不超过80字，解析不超过160字；公式写普通数学符号，不使用LaTeX反斜线。返回JSON：{\"schema\":\"mewu.practice/1\",\"items\":[{\"question\":\"题目\",\"answer\":\"答案\",\"explanation\":\"解析\",\"skill\":\"知识点\"}]}。只返回JSON，无answer根字段，正文用"+(LocalizationService.IsEnglish?"英文":"简体中文")+"。证据="+System.Text.Json.JsonSerializer.Serialize(issues);
        var draft=await Pass(prompt,false);token.ThrowIfCancellationRequested();
        var verified=await Pass("这是独立解题轮，没有提供初稿答案。按原顺序独立计算以下4道题，每一步合并系数和符号时重新计算，不能猜测答案。题干和skill必须保持不变，解析简短且完整，正文用"+(LocalizationService.IsEnglish?"英文":"简体中文")+"。只返回完整JSON：{\"schema\":\"mewu.practice/1\",\"items\":[{\"question\":\"原题干\",\"answer\":\"独立计算的答案\",\"explanation\":\"推导\",\"skill\":\"原知识点\"}]}，不要输出answer根字段。题目数据="+System.Text.Json.JsonSerializer.Serialize(new{schema="mewu.practice/1",items=draft.Select(i=>new{question=i.Question,skill=i.Skill})})+"\n允许的skill（严格原样使用）="+System.Text.Json.JsonSerializer.Serialize(skills),false);
        token.ThrowIfCancellationRequested();
        if(!draft.Select(i=>i.Question).SequenceEqual(verified.Select(i=>i.Question),StringComparer.Ordinal))throw new InvalidDataException(LocalizationService.T("复核改变了题干，请重新出题。","The verification changed the questions. Generate the exercises again."));
        return verified.Select((item,index)=>string.Concat(item.Answer.Where(c=>!char.IsWhiteSpace(c)))==string.Concat(draft[index].Answer.Where(c=>!char.IsWhiteSpace(c)))?item:item with{Explanation=LocalizationService.T("初稿与独立解答不一致，请重点验算。","The draft and independent answer differ. Check the calculation carefully.")+" "+item.Explanation}).ToArray();
        async Task<IReadOnlyList<PracticeItem>> Pass(string instruction,bool reasoning)
        {
            for(var attempt=0;attempt<2;attempt++)
            {
                try{var parsed=ParsePractice(await TeachingGradingService.SendAsync(provider,instruction+(attempt==0?"":"\n上一轮格式不完整或不合法，这是唯一一次重试。所有公式都是JSON字符串，使用普通数学符号，不要使用LaTeX反斜线转义或Markdown围栏，必须输出4个完整items，skill只使用提示中允许的名称。"),null,token,reasoning:reasoning));
                    if(parsed.Any(i=>!skills.Contains(i.Skill))||parsed.Select(i=>i.Question).Distinct(StringComparer.Ordinal).Count()!=4)throw new InvalidDataException("Exercise is duplicated or outside the reviewed skills");return parsed;}
                catch(Exception ex) when(attempt==0&&ex is System.Text.Json.JsonException or InvalidDataException or KeyNotFoundException or FormatException){token.ThrowIfCancellationRequested();}
            }
            throw new InvalidDataException("No complete exercises");
        }
    }
    internal static IReadOnlyList<PracticeItem> ParsePractice(string text)
    {
        using var doc=TeachingGradingService.ParseDocument(text);var root=doc.RootElement;
        if(TeachingGradingService.Text(root,"schema",40)!="mewu.practice/1")throw new InvalidDataException("Unknown exercise format");
        var items=root.GetProperty("items");if(items.ValueKind!=System.Text.Json.JsonValueKind.Array||items.GetArrayLength()!=4)throw new InvalidDataException("Expected four complete exercises");
        return items.EnumerateArray().Select(i=>new PracticeItem(Required(i,"question",2000),Required(i,"answer",1000),Required(i,"explanation",2000),Required(i,"skill",80))).ToArray();
        static string Required(System.Text.Json.JsonElement row,string field,int limit){var text=TeachingGradingService.Text(row,field,limit);return text.Length>0?text:throw new InvalidDataException("Empty exercise field");}
    }
    internal static string PracticeHtml(IReadOnlyList<PracticeItem> items,bool answers)=>"<!doctype html><html><head><meta charset=\"utf-8\"><title>"+(answers?"参考答案 / Answers":"巩固练习 / Practice")+"</title><style>body{font:18px 'Microsoft YaHei',sans-serif;max-width:800px;margin:40px auto;padding:24px;color:#182333}h1{font-size:28px}section{break-inside:avoid;margin:30px 0}pre{white-space:pre-wrap;font:inherit;line-height:1.8}.space{height:120px}@media print{body{margin:0}}</style></head><body><h1>"+(answers?"参考答案 / Answers":"巩固练习 / Practice")+"</h1>"+string.Concat(items.Select((i,n)=>$"<section><h2>{n+1}.</h2><pre>{WebUtility.HtmlEncode(i.Question)}</pre>"+(answers?$"<pre>{WebUtility.HtmlEncode(i.Answer)}\n{WebUtility.HtmlEncode(i.Explanation)}</pre>":"<div class=\"space\"></div>")+"</section>"))+"</body></html>";
    internal static void Export(string path,TeachingSession session,IReadOnlyList<(string Name,byte[] Data)> annotatedPages)
    {
        if(session.Practice.Count>0&&!session.PracticeConfirmed)throw new InvalidOperationException("Review exercises before export");
        if(session.Practice.Any(i=>string.IsNullOrWhiteSpace(i.Question)||string.IsNullOrWhiteSpace(i.Answer)||string.IsNullOrWhiteSpace(i.Explanation)))throw new InvalidDataException("Incomplete exercise");
        var full=Path.GetFullPath(path);var temporary=Path.Combine(Path.GetDirectoryName(full)!,".mewu-"+Guid.NewGuid().ToString("N")+".tmp");
        try
        {
            using(var stream=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None))
            using(var zip=new ZipArchive(stream,ZipArchiveMode.Create))
            {
                if(session.Practice.Count>0){Write("questions.html",PracticeHtml(session.Practice,false));Write("answers.html",PracticeHtml(session.Practice,true));}
                var csv=new StringBuilder("Submission,Page,Question,Observed,Expected,Verdict,Score,Maximum,Knowledge,Confirmed\r\n");
                foreach(var page in session.Pages)foreach(var item in page.Items)
                    csv.AppendLine(string.Join(',',new[]{page.Submission,page.PageNumber.ToString(),item.Question,item.Observed,item.Expected,TeachingSession.VerdictText(item.Verdict),item.Score?.ToString(System.Globalization.CultureInfo.InvariantCulture)??"",item.Maximum?.ToString(System.Globalization.CultureInfo.InvariantCulture)??"",item.Skill,item.Confirmed?"yes":"no"}.Select(Csv)));
                Write("review.csv",csv.ToString());
                foreach(var (name,data) in annotatedPages){using var target=zip.CreateEntry(name,CompressionLevel.Fastest).Open();target.Write(data);}
                void Write(string name,string text){using var writer=new StreamWriter(zip.CreateEntry(name).Open(),new UTF8Encoding(false));writer.Write(text);}
            }
            File.Move(temporary,full,true);
        }
        finally{if(File.Exists(temporary))File.Delete(temporary);}
    }
    private static string Csv(string value)=>"\""+((value.Length>0&&"=+-@\t\r".Contains(value[0]))?"'"+value:value).Replace("\"","\"\"")+"\"";
}
