using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class ProviderVerificationService
{
    private static byte[] CreateTestPng()
    {
        const int size=96;var pixels=new byte[size*size*4];
        for(var i=0;i<pixels.Length;i+=4){pixels[i]=248;pixels[i+1]=189;pixels[i+2]=56;pixels[i+3]=255;}
        var bitmap=System.Windows.Media.Imaging.BitmapSource.Create(size,size,96,96,System.Windows.Media.PixelFormats.Bgra32,null,pixels,size*4);
        var encoder=new System.Windows.Media.Imaging.PngBitmapEncoder();encoder.Frames.Add(System.Windows.Media.Imaging.BitmapFrame.Create(bitmap));
        using var stream=new MemoryStream();encoder.Save(stream);return stream.ToArray();
    }
    public async Task<string> VerifyAsync(AppSettings settings,CancellationToken token)
    {
        var provider=new AiProviderFactory().Create(settings);
        var connection=false;var text=false;var streaming=false;var image=false;var errors=new List<string>();
        if(provider is null)errors.Add("默认 Provider 或加密凭据不可用");
        else
        {
            try{connection=await provider.TestConnectionAsync(token);}catch(Exception ex){errors.Add("connection: "+ex.Message);}
            try
            {
                var streamed=new System.Text.StringBuilder();
                var result=await provider.SendAsync(new AiRequest{Prompt="只回复 MEWU_OK",StreamingProgress=new Progress<string>(x=>streamed.Append(x))},token);
                text=!string.IsNullOrWhiteSpace(result.Answer);streaming=streamed.Length>0;
            }catch(Exception ex){errors.Add("text: "+ex.Message);}
            try
            {
                var result=await provider.SendAsync(new AiRequest{Prompt="用一句中文说明这张图片的主要颜色。",Attachments=[new AiAttachment(AiAttachmentType.Image,"image/png",CreateTestPng())]},token);
                image=!string.IsNullOrWhiteSpace(result.Answer);
            }catch(Exception ex){errors.Add("image: "+ex.Message);}
        }
        var report=new{provider=provider?.Id,connection,text,streaming,image,errors,verifiedAt=DateTimeOffset.Now};
        var directory=Path.Combine(Path.GetTempPath(),"MewuAI");Directory.CreateDirectory(directory);
        var path=Path.Combine(directory,"provider-verification.json");
        await File.WriteAllTextAsync(path,JsonSerializer.Serialize(report,new JsonSerializerOptions{WriteIndented=true}),token);
        return path;
    }
}
