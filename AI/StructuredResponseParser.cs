using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StructuredResponseParser
{
    private static readonly Regex ThinkBlock = new("<(?<tag>think|thinking|reasoning)>\\s*(?<body>[\\s\\S]*?)\\s*</\\k<tag>>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex ThinkOpening = new("<(?:think|thinking|reasoning)>",RegexOptions.IgnoreCase|RegexOptions.Compiled);

    public static AiResult Parse(string value, string reasoning = "",bool expectStructuredResponse=false)
    {
        var extracted = ThinkBlock.Matches(value).Select(x => x.Groups["body"].Value.Trim()).Where(x => x.Length > 0);
        var allReasoning = string.Join(Environment.NewLine + Environment.NewLine, new[] { reasoning.Trim() }.Concat(extracted).Where(x => x.Length > 0));
        value = ThinkBlock.Replace(value, string.Empty).Trim();

        if (!TryGetStructuredPayload(value, out var json))
            return expectStructuredResponse&&LooksLikeBrokenStructuredPayload(value)
                ?new(string.Empty,[],allReasoning)
                :new(value, [], allReasoning);

        try
        {
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("answer", out var answerElement))
                return expectStructuredResponse?new(string.Empty,[],allReasoning):new(value, [], allReasoning);

            if (answerElement.ValueKind != JsonValueKind.String)
                return new(string.Empty, [], allReasoning);

            var answer = answerElement.GetString() ?? string.Empty;
            if (string.IsNullOrWhiteSpace(answer))
                return new(string.Empty, [], allReasoning);

            return new(answer, ParseAnnotations(root), allReasoning);
        }
        catch (JsonException)
        {
            return TryExtractAnswerFromTruncatedStructuredResponse(json, out var answer)
                ? new(answer, [], allReasoning)
                : expectStructuredResponse?new(string.Empty,[],allReasoning):new(value, [], allReasoning);
        }
    }

    private static bool LooksLikeBrokenStructuredPayload(string value)
    {
        var trimmed=value.TrimStart();
        if(trimmed.StartsWith('{')||trimmed.StartsWith('[')||trimmed.StartsWith("```",StringComparison.Ordinal))return true;
        if(!trimmed.StartsWith("json",StringComparison.OrdinalIgnoreCase))return false;
        var remainder=trimmed[4..].TrimStart();
        return remainder.StartsWith('{')||remainder.StartsWith('[');
    }

    private static IReadOnlyList<AiAnnotation> ParseAnnotations(JsonElement root)
    {
        if (!root.TryGetProperty("annotations", out var annotations) || annotations.ValueKind != JsonValueKind.Array)
            return [];

        var result = new List<AiAnnotation>();
        foreach (var item in annotations.EnumerateArray())
        {
            if (TryParseAnnotation(item, out var annotation))
                result.Add(annotation);
        }

        return result;
    }

    private static bool TryParseAnnotation(JsonElement item, out AiAnnotation annotation)
    {
        annotation = default!;
        if (item.ValueKind != JsonValueKind.Object ||
            !item.TryGetProperty("text", out var textElement) ||
            textElement.ValueKind != JsonValueKind.String)
            return false;

        var text = textElement.GetString() ?? string.Empty;
        var regionIndex = 0;
        if (item.TryGetProperty("regionIndex", out var regionElement) &&
            (regionElement.ValueKind != JsonValueKind.Number || !regionElement.TryGetInt32(out regionIndex)))
            return false;

        if (string.IsNullOrWhiteSpace(text) || regionIndex < 0)
            return false;

        var hasTimelineField=item.TryGetProperty("startTime",out _)||item.TryGetProperty("endTime",out _)||item.TryGetProperty("keyframes",out _);
        if(hasTimelineField)return TryParseVideoAnnotation(item,text,regionIndex,out annotation);

        if(!TryReadNormalizedRect(item,out var x,out var y,out var width,out var height))return false;

        annotation = new AiAnnotation(x, y, width, height, text, regionIndex);
        return true;
    }

    private static bool TryParseVideoAnnotation(JsonElement item,string text,int regionIndex,out AiAnnotation annotation)
    {
        annotation=default!;
        if(!TryReadFiniteDouble(item,"startTime",out var start)||
           !TryReadFiniteDouble(item,"endTime",out var end)||
           start<0||end<start||
           !item.TryGetProperty("keyframes",out var keyframesElement)||keyframesElement.ValueKind!=JsonValueKind.Array)return false;
        var keyframes=new List<VideoAnnotationKeyframe>();
        foreach(var keyframe in keyframesElement.EnumerateArray())
        {
            if(keyframe.ValueKind!=JsonValueKind.Object||
               !TryReadFiniteDouble(keyframe,"time",out var time)||time<start||time>end||
               !TryReadNormalizedRect(keyframe,out var x,out var y,out var width,out var height))return false;
            if(keyframes.Count>0&&time<=keyframes[^1].Time)return false;
            keyframes.Add(new(time,x,y,width,height));
        }
        if(keyframes.Count==0||(end>start&&keyframes.Count<2)||
           Math.Abs(keyframes[0].Time-start)>.001||Math.Abs(keyframes[^1].Time-end)>.001)return false;
        annotation=new AiAnnotation(keyframes[0].X,keyframes[0].Y,keyframes[0].Width,keyframes[0].Height,text,regionIndex,start,end,keyframes);
        return true;
    }

    private static bool TryReadNormalizedRect(JsonElement item,out double x,out double y,out double width,out double height)
    {
        x=y=width=height=0;
        if(!TryReadFiniteDouble(item,"x",out x)||!TryReadFiniteDouble(item,"y",out y)||
           !TryReadFiniteDouble(item,"width",out width)||!TryReadFiniteDouble(item,"height",out height))return false;
        return x>=0&&y>=0&&width>0&&height>0&&x+width<=1.001&&y+height<=1.001;
    }

    private static bool TryReadFiniteDouble(JsonElement item, string name, out double value)
    {
        value = 0;
        return item.TryGetProperty(name, out var element) &&
               element.ValueKind == JsonValueKind.Number &&
               element.TryGetDouble(out value) &&
               double.IsFinite(value);
    }

    public static string GetStreamingAnswerPreview(string value)
    {
        if(string.IsNullOrEmpty(value))return string.Empty;
        var payload=GetPartialStructuredPayload(value);
        if(payload.Length==0||!TryFindRootAnswerValue(payload.AsSpan(),out var answerStart))return string.Empty;
        var answerEnd=FindJsonStringEnd(payload.AsSpan(),answerStart);
        var encoded=answerEnd>=0
            ?payload.AsSpan(answerStart,answerEnd-answerStart)
            :payload.AsSpan(answerStart);
        if(!TryDecodeJsonStringPrefix(encoded,out var decoded))return string.Empty;
        return RemoveStreamingThinkContent(decoded);
    }

    public static string GetStreamingTextPreview(string value)
    {
        if(string.IsNullOrEmpty(value))return string.Empty;
        var trimmed=value.TrimStart();
        if(trimmed.StartsWith('{')||LooksLikeJsonFence(trimmed))return GetStreamingAnswerPreview(value);
        return RemoveStreamingThinkContent(value);
    }

    private static bool LooksLikeJsonFence(string value)
    {
        if(!value.StartsWith("```",StringComparison.Ordinal))return false;
        if(TryGetJsonFenceBodyStart(value,out _))return true;
        return "```json".StartsWith(value,StringComparison.OrdinalIgnoreCase);
    }

    private static bool TryGetStructuredPayload(string value, out string payload)
    {
        payload = string.Empty;
        var trimmed = value.Trim();
        if (trimmed.StartsWith('{'))
        {
            payload = trimmed;
            return true;
        }

        if (!trimmed.StartsWith("```", StringComparison.Ordinal))
            return false;

        if(!TryGetJsonFenceBodyStart(trimmed,out var bodyStart))return false;

        var body = trimmed[bodyStart..].Trim();
        if (body.EndsWith("```", StringComparison.Ordinal))
            body = body[..^3].TrimEnd();
        if (!body.StartsWith('{'))
            return false;

        payload = body;
        return true;
    }

    private static string GetPartialStructuredPayload(string value)
    {
        var trimmed=value.TrimStart();
        if(!trimmed.StartsWith("```",StringComparison.Ordinal))return trimmed;
        if(!TryGetJsonFenceBodyStart(trimmed,out var bodyStart))return string.Empty;
        return trimmed[bodyStart..].TrimStart();
    }

    private static bool TryGetJsonFenceBodyStart(string value,out int bodyStart)
    {
        bodyStart=0;
        if(!value.StartsWith("```",StringComparison.Ordinal)||value.Length<=3)return false;
        var index=3;
        if(value.AsSpan(index).StartsWith("json",StringComparison.OrdinalIgnoreCase))
        {
            index+=4;
            if(index<value.Length&&!char.IsWhiteSpace(value[index])&&value[index]!='{')return false;
        }
        else if(value[index]!='{'&&!char.IsWhiteSpace(value[index]))return false;
        while(index<value.Length&&char.IsWhiteSpace(value[index]))index++;
        if(index>=value.Length||value[index]!='{')return false;
        bodyStart=index;
        return true;
    }

    private static bool TryExtractAnswerFromTruncatedStructuredResponse(string json, out string answer)
    {
        answer = string.Empty;
        if (!TryFindRootAnswerValue(json.AsSpan(), out var answerStart))
            return false;
        if (!TryFindAnswerEndBeforeAnnotations(json.AsSpan(), answerStart, out var answerEnd))
            return false;
        if (!IsValidJsonPrefixWithEmptyAnswer(json, answerStart, answerEnd))
            return false;

        return TryDecodeJsonStringLoosely(json.AsSpan(answerStart, answerEnd - answerStart), out answer);
    }

    private static bool TryFindRootAnswerValue(ReadOnlySpan<char> json, out int answerStart)
    {
        answerStart = -1;
        var first = SkipWhitespace(json, 0);
        if (first >= json.Length || json[first] != '{')
            return false;

        var objectDepth = 0;
        var arrayDepth = 0;
        for (var index = first; index < json.Length; index++)
        {
            switch (json[index])
            {
                case '{':
                    objectDepth++;
                    break;
                case '}':
                    objectDepth--;
                    break;
                case '[':
                    arrayDepth++;
                    break;
                case ']':
                    arrayDepth--;
                    break;
                case '"':
                {
                    var stringEnd = FindJsonStringEnd(json, index + 1);
                    if (stringEnd < 0)
                        return false;

                    if (objectDepth == 1 && arrayDepth == 0)
                    {
                        var colon = SkipWhitespace(json, stringEnd + 1);
                        if (colon < json.Length && json[colon] == ':' &&
                            TryDecodeJsonStringLoosely(json[(index + 1)..stringEnd], out var propertyName) &&
                            propertyName == "answer")
                        {
                            var valueStart = SkipWhitespace(json, colon + 1);
                            if (valueStart >= json.Length || json[valueStart] != '"')
                                return false;

                            answerStart = valueStart + 1;
                            return true;
                        }
                    }

                    index = stringEnd;
                    break;
                }
            }
        }

        return false;
    }

    private static bool TryFindAnswerEndBeforeAnnotations(ReadOnlySpan<char> json, int answerStart, out int answerEnd)
    {
        answerEnd = -1;
        for (var index = answerStart; index < json.Length; index++)
        {
            if (json[index] == '\\')
            {
                index++;
                continue;
            }

            if (json[index] != '"')
                continue;

            var comma = SkipWhitespace(json, index + 1);
            if (comma >= json.Length || json[comma] != ',')
                continue;

            var propertyStart = SkipWhitespace(json, comma + 1);
            if (propertyStart >= json.Length || json[propertyStart] != '"')
                continue;

            var propertyEnd = FindJsonStringEnd(json, propertyStart + 1);
            if (propertyEnd < 0 || !TryDecodeJsonStringLoosely(json[(propertyStart + 1)..propertyEnd], out var propertyName) || propertyName != "annotations")
                continue;

            var colon = SkipWhitespace(json, propertyEnd + 1);
            if (colon >= json.Length || json[colon] != ':')
                continue;

            var annotationStart = SkipWhitespace(json, colon + 1);
            if (!LooksLikeAnnotationValuePrefix(json, annotationStart))
                continue;

            answerEnd = index;
            return true;
        }

        return false;
    }

    private static bool LooksLikeAnnotationValuePrefix(ReadOnlySpan<char> json, int start)
    {
        if (start >= json.Length)
            return true;
        if (json[start] == '[')
            return true;

        const string nullLiteral = "null";
        var remaining = json[start..];
        var prefixLength = Math.Min(remaining.Length, nullLiteral.Length);
        return prefixLength > 0 && remaining[..prefixLength].SequenceEqual(nullLiteral.AsSpan(0, prefixLength));
    }

    private static bool IsValidJsonPrefixWithEmptyAnswer(string json, int answerStart, int answerEnd)
    {
        var normalized = string.Concat(json.AsSpan(0, answerStart), json.AsSpan(answerEnd));
        var utf8 = Encoding.UTF8.GetBytes(normalized);
        var reader = new Utf8JsonReader(utf8, isFinalBlock: false, state: default);
        try
        {
            while (reader.Read())
            {
            }

            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static int FindJsonStringEnd(ReadOnlySpan<char> value, int start)
    {
        for (var index = start; index < value.Length; index++)
        {
            if (value[index] == '\\')
            {
                index++;
                continue;
            }

            if (value[index] == '"')
                return index;
        }

        return -1;
    }

    private static bool TryDecodeJsonStringLoosely(ReadOnlySpan<char> value, out string decoded)
    {
        var builder = new StringBuilder(value.Length);
        for (var index = 0; index < value.Length; index++)
        {
            var character = value[index];
            if (character != '\\')
            {
                builder.Append(character);
                continue;
            }

            if (++index >= value.Length)
            {
                decoded = string.Empty;
                return false;
            }

            switch (value[index])
            {
                case '"': builder.Append('"'); break;
                case '\\': builder.Append('\\'); break;
                case '/': builder.Append('/'); break;
                case 'b': builder.Append('\b'); break;
                case 'f': builder.Append('\f'); break;
                case 'n': builder.Append('\n'); break;
                case 'r': builder.Append('\r'); break;
                case 't': builder.Append('\t'); break;
                case 'u':
                    if (index + 4 >= value.Length || !ushort.TryParse(value.Slice(index + 1, 4), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var codePoint))
                    {
                        decoded = string.Empty;
                        return false;
                    }

                    builder.Append((char)codePoint);
                    index += 4;
                    break;
                default:
                    decoded = string.Empty;
                    return false;
            }
        }

        decoded = builder.ToString();
        return true;
    }

    private static bool TryDecodeJsonStringPrefix(ReadOnlySpan<char> value,out string decoded)
    {
        var builder=new StringBuilder(value.Length);
        for(var index=0;index<value.Length;index++)
        {
            var character=value[index];
            if(character!='\\')
            {
                if(character<' '){decoded=string.Empty;return false;}
                builder.Append(character);
                continue;
            }

            if(index+1>=value.Length)break;
            var escape=value[++index];
            switch(escape)
            {
                case '"':builder.Append('"');break;
                case '\\':builder.Append('\\');break;
                case '/':builder.Append('/');break;
                case 'b':builder.Append('\b');break;
                case 'f':builder.Append('\f');break;
                case 'n':builder.Append('\n');break;
                case 'r':builder.Append('\r');break;
                case 't':builder.Append('\t');break;
                case 'u':
                {
                    if(index+4>=value.Length)goto CompletePrefix;
                    if(!ushort.TryParse(value.Slice(index+1,4),NumberStyles.AllowHexSpecifier,CultureInfo.InvariantCulture,out var codePoint)){decoded=string.Empty;return false;}
                    index+=4;
                    if(char.IsHighSurrogate((char)codePoint))
                    {
                        if(index+6>=value.Length||value[index+1]!='\\'||value[index+2]!='u')goto CompletePrefix;
                        if(!ushort.TryParse(value.Slice(index+3,4),NumberStyles.AllowHexSpecifier,CultureInfo.InvariantCulture,out var low)||!char.IsLowSurrogate((char)low)){decoded=string.Empty;return false;}
                        builder.Append((char)codePoint);builder.Append((char)low);index+=6;
                    }
                    else if(char.IsLowSurrogate((char)codePoint)){decoded=string.Empty;return false;}
                    else builder.Append((char)codePoint);
                    break;
                }
                default:decoded=string.Empty;return false;
            }
        }

CompletePrefix:
        decoded=builder.ToString();
        return true;
    }

    private static string RemoveStreamingThinkContent(string value)
    {
        var visible=ThinkBlock.Replace(value,string.Empty);
        var opening=ThinkOpening.Match(visible);
        if(opening.Success)visible=visible[..opening.Index];
        var partialStart=visible.LastIndexOf('<');
        if(partialStart>=0)
        {
            var partial=visible[partialStart..];
            var tags=new[]{"<think","<thinking","<reasoning","</think","</thinking","</reasoning"};
            if(tags.Any(tag=>tag.StartsWith(partial,StringComparison.OrdinalIgnoreCase)))visible=visible[..partialStart];
        }
        return visible;
    }

    private static int SkipWhitespace(ReadOnlySpan<char> value, int start)
    {
        while (start < value.Length && char.IsWhiteSpace(value[start]))
            start++;
        return start;
    }
}
