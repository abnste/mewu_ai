// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;
using System.Text;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StreamingResponseParser
{
    public static bool TryParse(string line,out AiStreamDelta delta,out bool done)
        =>TryParse(line,out delta,out done,out _);

    public static bool TryParse(string line,out AiStreamDelta delta,out bool done,out bool truncated)
    {
        delta=new(string.Empty,string.Empty);done=false;truncated=false;
        if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))return false;
        var payload=line[5..].Trim();if(payload=="[DONE]"){done=true;return true;}
        try
        {
            using var document=JsonDocument.Parse(payload);var choices=document.RootElement.GetProperty("choices");if(choices.GetArrayLength()==0)return true;
            var choice=choices[0];done=choice.TryGetProperty("finish_reason",out var finish)&&finish.ValueKind==JsonValueKind.String&&!string.IsNullOrWhiteSpace(finish.GetString());
            truncated=done&&string.Equals(finish.GetString(),"length",StringComparison.OrdinalIgnoreCase);
            if(!choice.TryGetProperty("delta",out var value)||value.ValueKind!=JsonValueKind.Object)return done;
            var (content,typedReasoning)=ReadContentParts(value);var reasoning=ReadString(value,"reasoning_content");var cumulative=false;
            if(reasoning.Length==0)reasoning=ReadString(value,"thinking_content");
            if(reasoning.Length==0)reasoning=ReadString(value,"reasoning");
            if(reasoning.Length==0)reasoning=typedReasoning;
            if(reasoning.Length==0){reasoning=ReadReasoningDetails(value);cumulative=reasoning.Length>0;}
            delta=new(content,reasoning,cumulative);return true;
        }
        catch(JsonException){return false;}
        catch(KeyNotFoundException){return false;}
        catch(InvalidOperationException){return false;}
    }

    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;

    internal static (string Content,string Reasoning) ReadContentParts(JsonElement value)
    {
        if(value.ValueKind!=JsonValueKind.Object)return (string.Empty,string.Empty);
        if(!value.TryGetProperty("content",out var content))
        {
            // A few OpenAI-compatible gateways put their final text in these
            // aliases.  They are accepted only as explicit text fields; no
            // reasoning field is ever promoted into an answer.
            var outputText=ReadString(value,"output_text");
            if(outputText.Length==0)outputText=ReadString(value,"text");
            return (outputText,string.Empty);
        }
        if(content.ValueKind==JsonValueKind.String)return (content.GetString()??string.Empty,string.Empty);
        if(content.ValueKind==JsonValueKind.Object)
        {
            // A gateway may wrap an otherwise plain answer in a single object.
            // Only an absent discriminator permits this compatibility path;
            // unknown or malformed declared types must never become answer text.
            if(!content.TryGetProperty("type",out _))
            {
                var objectText=ReadString(content,"text");
                if(objectText.Length==0)objectText=ReadString(content,"content");
                return (objectText,string.Empty);
            }
            var objectTextParts=new StringBuilder();
            var objectReasoningParts=new StringBuilder();
            AppendTypedContent(content,objectTextParts,objectReasoningParts);
            return (objectTextParts.ToString(),objectReasoningParts.ToString());
        }
        if(content.ValueKind!=JsonValueKind.Array)return (string.Empty,string.Empty);

        var text=new StringBuilder();
        var reasoning=new StringBuilder();
        foreach(var chunk in content.EnumerateArray())
            AppendTypedContent(chunk,text,reasoning);
        return (text.ToString(),reasoning.ToString());
    }

    private static void AppendTypedContent(JsonElement chunk,StringBuilder text,StringBuilder reasoning)
    {
        if(chunk.ValueKind!=JsonValueKind.Object)return;
        switch(ReadString(chunk,"type"))
        {
            case "text":
            case "output_text":
            case "text_delta":
                // Text content blocks, including Claude's text_delta, use text.
                // A Responses response.output_text.delta event is a different
                // envelope and must not be inferred from a generic delta field.
                text.Append(ReadString(chunk,"text"));
                break;
            case "thinking":
                // Mistral streams ThinkChunk.thinking as TextChunk[], including
                // a mixed thinking/text event when the final answer begins.
                if(!chunk.TryGetProperty("thinking",out var thoughts)||thoughts.ValueKind!=JsonValueKind.Array)break;
                foreach(var thought in thoughts.EnumerateArray())
                    if(thought.ValueKind==JsonValueKind.Object&&ReadString(thought,"type")=="text")
                        reasoning.Append(ReadString(thought,"text"));
                break;
        }
    }

    internal static string ReadReasoningDetails(JsonElement value)
    {
        if(!value.TryGetProperty("reasoning_details",out var details))return string.Empty;
        if(details.ValueKind==JsonValueKind.String)return details.GetString()??string.Empty;
        if(details.ValueKind!=JsonValueKind.Array)return string.Empty;
        return string.Concat(details.EnumerateArray().Select(item=>item.ValueKind==JsonValueKind.Object?ReadString(item,"text"):string.Empty));
    }
}
