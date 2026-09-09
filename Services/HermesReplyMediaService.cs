// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Text.RegularExpressions;
using Markdig;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class HermesReplyMediaService
{
    private static readonly Regex MediaLine=new(@"^[\t ]*MEDIA:[\t ]*(?<source>[^\r\n]+)[\t ]*$",RegexOptions.Multiline|RegexOptions.CultureInvariant,TimeSpan.FromMilliseconds(100));

    internal static IReadOnlyList<string> ReadGeneratedImages(JsonElement payload)
    {
        if(payload.ValueKind!=JsonValueKind.Object||!payload.TryGetProperty("name",out var name)||name.ValueKind!=JsonValueKind.String||name.GetString()!="image_generate"||!payload.TryGetProperty("result",out var result))return [];
        JsonDocument? nested=null;
        try
        {
            if(result.ValueKind==JsonValueKind.String)
            {
                var text=result.GetString();if(text is null||text.Length>256*1024)return [];
                nested=JsonDocument.Parse(text,new JsonDocumentOptions{MaxDepth=16});result=nested.RootElement;
            }
            if(result.ValueKind!=JsonValueKind.Object||!result.TryGetProperty("success",out var success)||success.ValueKind!=JsonValueKind.True)return [];
            var images=new List<string>();
            Add(result,"image");
            if(result.TryGetProperty("images",out var list)&&list.ValueKind==JsonValueKind.Array)
                foreach(var item in list.EnumerateArray().Take(16))
                {
                    if(item.ValueKind==JsonValueKind.String)AddSource(item.GetString());
                    else if(item.ValueKind==JsonValueKind.Object){Add(item,"url");Add(item,"path");}
                }
            return images;
            void Add(JsonElement item,string key){if(item.TryGetProperty(key,out var value)&&value.ValueKind==JsonValueKind.String)AddSource(value.GetString());}
            void AddSource(string? source){if(images.Count<16&&TryNormalizeSource(source,out var normalized,out _))images.Add(normalized);}
        }
        catch(JsonException){return [];}
        finally{nested?.Dispose();}
    }

    internal static AiResult Complete(AiResult result,IReadOnlyList<string> generatedImages)
    {
        var answer=result.Answer;
        var parsed=Markdown.Parse(answer);
        var seen=new HashSet<string>(StringComparer.Ordinal);
        var local=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var append=new List<string>();
        foreach(var link in parsed.Descendants<LinkInline>().Where(link=>link.IsImage).Take(16))Register(link.Url,false);
        var code=parsed.Descendants<CodeBlock>().Select(block=>block.Span).ToArray();
        answer=MediaLine.Replace(answer,match=>
        {
            if(code.Any(span=>match.Index>=span.Start&&match.Index<=span.End))return match.Value;
            return Register(match.Groups["source"].Value,true)?string.Empty:match.Value;
        });
        // Hermes sometimes returns a download link instead of Markdown image syntax.
        foreach(var link in parsed.Descendants<LinkInline>().Where(link=>!link.IsImage).Take(32))
            if(ReplyImageService.TryGetLocalPath(link.Url,out _))Register(link.Url,true);
        foreach(var image in generatedImages.Take(16))Register(image,true);
        if(append.Count>0)answer=answer.TrimEnd()+"\n\n"+string.Join("\n\n",append.Select(source=>$"![Hermes 生成的图片](<{source}>)"));
        return result with {Answer=answer,LocalReplyImageSources=local.ToArray()};

        bool Register(string? source,bool add)
        {
            if(!TryNormalizeSource(source,out var normalized,out var path))return false;
            if(seen.Contains(normalized))return true;
            if(seen.Count>=16)return false;
            seen.Add(normalized);if(path is not null)local.Add(path);
            if(add)append.Add(normalized);
            return true;
        }
    }

    private static bool TryNormalizeSource(string? source,out string normalized,out string? local)
    {
        normalized=string.Empty;local=null;
        var value=source?.Trim().Trim('"','\'','`','<','>');
        if(ReplyImageService.TryGetLocalPath(value,out var path)){local=path;normalized=new Uri(path).AbsoluteUri;return true;}
        if(ReplyImageService.TryGetWebUri(value,out var uri)){normalized=uri.AbsoluteUri;return true;}
        return false;
    }
}
