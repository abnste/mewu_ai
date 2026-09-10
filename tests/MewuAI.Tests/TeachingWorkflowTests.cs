// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Services;
using mewu_ai_Assistant.Models;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Xunit;

namespace MewuAI.Tests;

public sealed class TeachingWorkflowTests
{
    [Theory]
    [InlineData("= x¹²y⁻¹⁵ / (xy²) = x¹³y⁻¹⁷ = x¹³/y¹⁷","= x¹²y⁻¹⁵ / (xy²)\n= x¹³y⁻¹⁷\n= x¹³/y¹⁷")]
    [InlineData("a = b = c","a = b\n= c")]
    [InlineData("x = 2","x = 2")]
    [InlineData("= a\n= b","= a\n= b")]
    [InlineData("(a = b) = (c = d)","(a = b) = (c = d)")]
    [InlineData("x = 1, y = 2","x = 1, y = 2")]
    [InlineData("a == b == c","a == b == c")]
    [InlineData("x = 1 → y = 2","x = 1 → y = 2")]
    [InlineData("a = [b) = c","a = [b) = c")]
    public void CalculationLayoutPreservesSymbolsAndExplicitLines(string input,string expected)
    {
        var result=TeachingCalculationLayout.Format(input);Assert.Equal(expected,result);
        Assert.Equal(string.Concat(input.Where(c=>!char.IsWhiteSpace(c))),string.Concat(result.Where(c=>!char.IsWhiteSpace(c))));
    }
    [Fact] public void GradingJsonPreservesOriginalStepBreaks()
    {
        const string steps="= x¹²y⁻¹⁵ / (xy²)\n= x¹³y⁻¹⁷\n= x¹³/y¹⁷";
        Assert.Equal(steps,TeachingGradingService.Parse(Response(observed:steps),"page-a",false)[0].Observed);
    }
    [Fact] public void ReviewExportKeepsStepBreaksAndCorrectedExplanation()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuStepExport-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(directory);
        try
        {
            var image=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[16],8);image.Freeze();
            var row=TeachingGradingService.Parse(Response(observed:"= a\n= b\n= c"),"page-a",false)[0] with{Reason="Check the denominator.\nKeep the plus sign.",Confirmed=true};
            var page=new TeachingPage("a","A",1,image,"a"){Items=[row]};var session=new TeachingSession();session.AddRange([page]);
            var path=Path.Combine(directory,"review.zip");TeachingExportService.Export(path,session,[]);
            using var zip=System.IO.Compression.ZipFile.OpenRead(path);using var reader=new StreamReader(zip.GetEntry("review.csv")!.Open());var csv=reader.ReadToEnd();
            Assert.Contains("Knowledge,Reason,Confirmed",csv);Assert.Contains("\"'= a\n= b\n= c\"",csv);Assert.Contains("\"Check the denominator.\nKeep the plus sign.\"",csv);
        }
        finally{Directory.Delete(directory,true);}
    }
    private static string Response(string verdict="incorrect",string observed="5.6e4",decimal? score=null,decimal? maximum=null)=>JsonSerializer.Serialize(new
    {
        schema="mewu.grading/1",pageId="page-a",complete=true,items=new[]{new{question="26",observed,expected="5.6e-4",verdict,reason="Exponent sign",skill="Scientific notation",rect=new[]{.6,.2,.2,.1},score,maximum}}
    });
    [Fact] public void PreservesMathSignsAndRejectsCrossPageResults()
    {
        var rows=TeachingGradingService.Parse(Response(),"page-a",false);Assert.Equal("5.6e4",rows[0].Observed);Assert.Equal("5.6e-4",rows[0].Expected);
        Assert.Throws<InvalidDataException>(()=>TeachingGradingService.Parse(Response(),"other",false));
    }
    [Fact] public void DisagreementBecomesUncertainAndCannotCarryScore()
    {
        var first=TeachingGradingService.Parse(Response(score:0,maximum:2),"page-a",true);
        var second=TeachingGradingService.Parse(Response("correct","5.6e-4",2,2),"page-a",true);
        var reconciled=Assert.Single(TeachingGradingService.Reconcile(first,second));Assert.Equal(GradingVerdict.Uncertain,reconciled.Verdict);Assert.Null(reconciled.Score);Assert.False(reconciled.Confirmed);
    }
    [Fact] public void MissingQuestionInVerificationIsNotSilentlyDropped()
    {
        var first=TeachingGradingService.Parse(Response(),"page-a",false);var merged=TeachingGradingService.Reconcile(first,[]);Assert.Single(merged);Assert.Equal(GradingVerdict.Uncertain,merged[0].Verdict);
    }
    [Fact] public void NoRubricMeansNoInventedScore()
    {
        var row=Assert.Single(TeachingGradingService.Parse(Response(score:4,maximum:6),"page-a",false));Assert.Null(row.Score);Assert.Null(row.Maximum);
        Assert.Throws<InvalidDataException>(()=>TeachingGradingService.Parse(Response(score:7,maximum:6),"page-a",true));
    }
    [Fact] public void RejectsTruncationUnknownSchemaDuplicateQuestionAndInvalidBounds()
    {
        Assert.ThrowsAny<JsonException>(()=>TeachingGradingService.Parse(Response()[..^2],"page-a",false));
        var root=JsonSerializer.Deserialize<Dictionary<string,JsonElement>>(Response())!;
        root["schema"]=JsonSerializer.SerializeToElement("mewu.grading/2");Assert.Throws<InvalidDataException>(()=>TeachingGradingService.Parse(JsonSerializer.Serialize(root),"page-a",false));
        root["schema"]=JsonSerializer.SerializeToElement("mewu.grading/1");root["items"]=JsonSerializer.SerializeToElement(new[]{root["items"][0],root["items"][0]});Assert.Throws<InvalidDataException>(()=>TeachingGradingService.Parse(JsonSerializer.Serialize(root),"page-a",false));
        Assert.Throws<InvalidDataException>(()=>TeachingGradingService.Parse(Response("blank","something"),"page-a",false));
    }
    [Theory]
    [InlineData("3,5-7",20,new[]{3,5,6,7})]
    [InlineData("2,2,1",3,new[]{1,2})]
    public void PdfPageRangesAreExplicitAndBounded(string range,int total,int[] expected)=>Assert.Equal(expected,TeachingImportService.ParsePages(range,total));
    [Theory][InlineData("0")][InlineData("3-1")][InlineData("1-17")][InlineData("99999")][InlineData("1,,2")]
    public void InvalidPageRangesFail(string range)=>Assert.Throws<InvalidDataException>(()=>TeachingImportService.ParsePages(range,30));
    [Fact] public void PracticeExportKeepsAnswersSeparateAndEscapesHtml()
    {
        PracticeItem[] items=[new("<script> x + 1?","SECRET_ANSWER","Explanation","Algebra")];
        var questions=TeachingExportService.PracticeHtml(items,false);Assert.DoesNotContain("SECRET_ANSWER",questions);Assert.DoesNotContain("<script>",questions);Assert.Contains("&lt;script&gt;",questions);
        Assert.Contains("SECRET_ANSWER",TeachingExportService.PracticeHtml(items,true));
    }
    [Fact] public void UnknownAndIncompletePracticeAreRejected()
    {
        Assert.Throws<InvalidDataException>(()=>TeachingExportService.ParsePractice("{\"schema\":\"mewu.practice/1\",\"items\":[]}"));
    }
    [Fact] public void SameStudentAcrossPagesDoesNotBecomeTwoStudentsAndUnreviewedRowsAreExcluded()
    {
        var image=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[16],8);image.Freeze();
        var a=new TeachingPage("a","A",1,image,"hash-a"){Items=TeachingGradingService.Parse(Response(),"page-a",false)};
        var next=new TeachingPage("a2","A",2,image,"hash-a2"){Items=TeachingGradingService.Parse(Response(),"page-a",false)};
        var b=new TeachingPage("b","B",1,image,"hash-b"){Items=TeachingGradingService.Parse(Response(),"page-a",false)};
        var session=new TeachingSession();session.AddRange([a,next,b]);a.Items[0].Confirmed=true;next.Items[0].Confirmed=true;Assert.Empty(session.CommonIssues());
        b.Items[0].Confirmed=true;var issue=Assert.Single(session.CommonIssues());Assert.Equal(2,issue.Submissions);
        Assert.Throws<InvalidOperationException>(()=>session.AddRange([new("duplicate","A",3,image,"hash-a")]));Assert.Equal(3,session.Pages.Count);
    }
    [Fact] public async Task CancellationAfterProviderReturnsRejectsResultAndWipesOwnedImages()
    {
        var image=BitmapSource.Create(2,2,96,96,PixelFormats.Bgra32,null,new byte[16],8);image.Freeze();
        var page=new TeachingPage("page-a","A",1,image,"a");using var canceled=new CancellationTokenSource();
        var provider=new FakeProvider(request=>{Assert.Null(request.StreamingProgress);canceled.Cancel();return new(Response(),[]);});
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>new TeachingGradingService().GradeAsync(provider,page,"",null,canceled.Token));
        Assert.Single(provider.Buffers);Assert.All(provider.Buffers[0],value=>Assert.Equal(0,value));Assert.Empty(page.Items);
    }
    private static string PracticeResponse(string skill="Algebra")=>JsonSerializer.Serialize(new{schema="mewu.practice/1",items=Enumerable.Range(1,4).Select(n=>new{question=$"{n} + 1 = ?",answer=(n+1).ToString(),explanation=$"Add one to {n}",skill})});
    [Fact] public async Task PracticeRetriesOnlyTheInvalidPassAndChecksTheReturnedSkills()
    {
        var calls=0;
        var provider=new FakeProvider(request=>{calls++;if(calls>=3)Assert.DoesNotContain("Add one to",request.Prompt);return new(calls switch{1=>PracticeResponse("Unrelated"),2=>PracticeResponse(),3=>"{",_=>PracticeResponse()},[]);});
        var result=await TeachingExportService.CreatePracticeAsync(provider,[new("Algebra",["A / 1","B / 1"],2)],CancellationToken.None);
        Assert.Equal(4,calls);Assert.Equal(4,result.Count);Assert.All(result,item=>Assert.Equal("Algebra",item.Skill));
    }
    [Fact] public async Task PersistentlyMalformedPracticeStopsAfterOneRetry()
    {
        var calls=0;var provider=new FakeProvider(request=>{calls++;return new("{",[]);});
        await Assert.ThrowsAnyAsync<JsonException>(()=>TeachingExportService.CreatePracticeAsync(provider,[new("Algebra",["A / 1","B / 1"],2)],CancellationToken.None));
        Assert.Equal(2,calls);
    }
    [Fact] public async Task PracticeCancellationDoesNotStartVerificationOrAcceptTheDraft()
    {
        using var cancellation=new CancellationTokenSource();var calls=0;
        var provider=new FakeProvider(request=>{calls++;cancellation.Cancel();return new(PracticeResponse(),[]);});
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>TeachingExportService.CreatePracticeAsync(provider,[new("Algebra",["A / 1","B / 1"],2)],cancellation.Token));
        Assert.Equal(1,calls);
    }
    private sealed class FakeProvider(Func<AiRequest,AiResult> send):IAiProvider
    {
        public string Id=>"test";
        public AiProviderCapabilities Capabilities=>new(true,false,true,8*1024*1024,0,TimeSpan.Zero,new HashSet<string>{"image/png"});
        internal List<byte[]> Buffers { get; }=[];
        public Task<bool> TestConnectionAsync(CancellationToken cancellationToken)=>Task.FromResult(true);
        public Task<AiResult> SendAsync(AiRequest request,CancellationToken cancellationToken){Buffers.AddRange(request.Attachments.Select(a=>a.Data!));return Task.FromResult(send(request));}
    }
    [Fact] public void UniqueAnswerOcrCanFixDistantModelBoxWithoutMatchingQuestionText()
    {
        var row=TeachingGradingService.Parse(Response(observed:"-5"),"page-a",false)[0];
        var document=new OcrDocument("",[new OcrLine("25. (-5)^(-1)",20,100,200,30,[]),new OcrLine("-5",500,500,35,30,[])]);
        var refined=Assert.Single(TeachingAnswerGeometry.Refine(document,1000,1000,[row]));Assert.Equal(.5,refined.Y);Assert.Equal(.035,refined.Width);Assert.Equal(row.Verdict,refined.Verdict);
        Assert.NotEqual(TeachingAnswerGeometry.Normalize("10^-4"),TeachingAnswerGeometry.Normalize("10^4"));Assert.Equal(TeachingAnswerGeometry.Normalize("4x²+1"),TeachingAnswerGeometry.Normalize("4x^2 + 1"));
    }
    [Fact] public void TeachingPackRequiresExerciseReviewAndDoesNotLeakAnswersIntoQuestions()
    {
        var directory=Path.Combine(Path.GetTempPath(),"MewuTeachingTest-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(directory);
        try
        {
            var path=Path.Combine(directory,"pack.zip");var session=new TeachingSession{Practice=[new("Question?","ANSWER_ONLY","Proof","Skill")]};
            Assert.Throws<InvalidOperationException>(()=>TeachingExportService.Export(path,session,[]));Assert.False(File.Exists(path));
            session.PracticeConfirmed=true;TeachingExportService.Export(path,session,[]);
            using var zip=System.IO.Compression.ZipFile.OpenRead(path);using var questions=new StreamReader(zip.GetEntry("questions.html")!.Open());using var answers=new StreamReader(zip.GetEntry("answers.html")!.Open());
            Assert.DoesNotContain("ANSWER_ONLY",questions.ReadToEnd());Assert.Contains("ANSWER_ONLY",answers.ReadToEnd());Assert.NotNull(zip.GetEntry("review.csv"));Assert.DoesNotContain(Directory.GetFiles(directory),f=>f.EndsWith(".tmp",StringComparison.Ordinal));
        }
        finally{Directory.Delete(directory,true);}
    }
}
