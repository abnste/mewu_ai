using System.Text.Json;
using mewu_ai_Assistant.AI;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class ProviderVerificationService
{
    private readonly AiProviderFactory _factory;
    private readonly string _reportPath;

    public ProviderVerificationService()
        :this(
            new AiProviderFactory(),
            Path.Combine(Path.GetTempPath(),"MewuAI","provider-verification.json")){}

    internal ProviderVerificationService(AiProviderFactory factory,string reportPath)
    {
        _factory=factory??throw new ArgumentNullException(nameof(factory));
        _reportPath=Path.GetFullPath(reportPath??throw new ArgumentNullException(nameof(reportPath)));
    }

    private static byte[] CreateTestPng()
    {
        // A 96px near-blue square was occasionally downsampled by M3's vision
        // preprocessor into an effectively blank thumbnail.  A 256px pure-blue
        // probe stays tiny on the wire while remaining unambiguous after the
        // provider's documented image preprocessing.
        const int size=256;var pixels=new byte[size*size*4];
        for(var i=0;i<pixels.Length;i+=4){pixels[i]=255;pixels[i+1]=0;pixels[i+2]=0;pixels[i+3]=255;}
        var bitmap=System.Windows.Media.Imaging.BitmapSource.Create(size,size,96,96,System.Windows.Media.PixelFormats.Bgra32,null,pixels,size*4);
        var encoder=new System.Windows.Media.Imaging.PngBitmapEncoder();encoder.Frames.Add(System.Windows.Media.Imaging.BitmapFrame.Create(bitmap));
        using var stream=new MemoryStream();encoder.Save(stream);return stream.ToArray();
    }
    public async Task<string> VerifyAsync(AppSettings settings,CancellationToken token)
    {
        return (await VerifyCoreAsync(settings,token)).Path;
    }

    public async Task<string> VerifyRequiredAsync(AppSettings settings,CancellationToken token)
    {
        var run=await VerifyCoreAsync(settings,token);
        if(!run.Succeeded)
        {
            throw new ProviderVerificationException(
                $"Provider 验证未全部通过：{string.Join("；",run.Failures)}。报告：{run.Path}",
                run.Path);
        }
        return run.Path;
    }

    private async Task<VerificationRun> VerifyCoreAsync(AppSettings settings,CancellationToken token)
    {
        var provider=_factory.Create(settings,out var providerError);
        var connection=false;var text=false;var streaming=false;var image=false;var errors=new List<string>();
        if(provider is null)errors.Add(providerError??"默认 Provider 或加密凭据不可用");
        else
        {
            try{connection=await provider.TestConnectionAsync(token);}catch(Exception ex)when(ex is not OperationCanceledException){errors.Add("connection: "+ex.Message);}
            try
            {
                var streamed=new System.Text.StringBuilder();
                var result=await provider.SendAsync(new AiRequest{Prompt=$"只回复 {OpenAiCompatibleProvider.ConnectionProbeMarker}，不要添加其他内容。",StreamingProgress=new InlineProgress<AiStreamDelta>(x=>streamed.Append(x.Content))},token);
                text=MatchesTextProbe(result.Answer);streaming=MatchesTextProbe(streamed.ToString());
            }catch(Exception ex)when(ex is not OperationCanceledException){errors.Add("text: "+ex.Message);}
            try
            {
                var result=await provider.SendAsync(new AiRequest{Prompt="识别图片的主要颜色：如果主要是蓝色，只回复 MEWU_BLUE；否则只回复 MEWU_OTHER。",Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",CreateTestPng())],DisableReasoning=true,MaxOutputTokens=32},token);
                image=MatchesImageProbe(result.Answer);
            }catch(Exception ex)when(ex is not OperationCanceledException){errors.Add("image: "+ex.Message);}
        }
        if(!connection&&!errors.Any(error=>error.StartsWith("connection:",StringComparison.Ordinal)))errors.Add("connection: 连接测试未通过");
        if(!text&&!errors.Any(error=>error.StartsWith("text:",StringComparison.Ordinal)))errors.Add("text: 正文未返回 MEWU_OK 校验标记");
        if(!streaming&&!errors.Any(error=>error.StartsWith("text:",StringComparison.Ordinal)))errors.Add("streaming: 流式正文未返回 MEWU_OK 校验标记");
        if(!image&&!errors.Any(error=>error.StartsWith("image:",StringComparison.Ordinal)))errors.Add("image: 未识别出测试图的蓝色校验标记");
        var succeeded=connection&&text&&streaming&&image;
        var report=new{provider=provider?.Id,connection,text,streaming,image,succeeded,errors,verifiedAt=DateTimeOffset.Now};
        var directory=Path.GetDirectoryName(_reportPath)??throw new InvalidOperationException("Provider 验证报告目录无效");Directory.CreateDirectory(directory);
        await File.WriteAllTextAsync(_reportPath,JsonSerializer.Serialize(report,new JsonSerializerOptions{WriteIndented=true}),token);
        return new(_reportPath,succeeded,errors);
    }

    internal static bool MatchesTextProbe(string? value)=>OpenAiCompatibleProvider.MatchesConnectionProbe(value);
    internal static bool MatchesImageProbe(string? value)=>MatchesProbe(value,"MEWU_BLUE");
    private static bool MatchesProbe(string? value,string expected)=>string.Equals(value?.Trim(),expected,StringComparison.OrdinalIgnoreCase);

    private sealed class InlineProgress<T>(Action<T> handler):IProgress<T>
    {
        public void Report(T value)=>handler(value);
    }

    private sealed record VerificationRun(string Path,bool Succeeded,IReadOnlyList<string> Failures);
}

public sealed class ProviderVerificationException(string message,string reportPath):InvalidOperationException(message)
{
    public string ReportPath { get; }=reportPath;
}
