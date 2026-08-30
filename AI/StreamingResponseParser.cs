using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StreamingResponseParser
{
    public static bool TryParse(string line,out AiStreamDelta delta,out bool done)
    {
        delta=new(string.Empty,string.Empty);done=false;
        if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))return false;
        var payload=line[5..].Trim();if(payload=="[DONE]"){done=true;return true;}
        try
        {
            using var document=JsonDocument.Parse(payload);var choices=document.RootElement.GetProperty("choices");if(choices.GetArrayLength()==0)return true;
            var choice=choices[0];done=choice.TryGetProperty("finish_reason",out var finish)&&finish.ValueKind==JsonValueKind.String&&!string.IsNullOrWhiteSpace(finish.GetString());
            if(!choice.TryGetProperty("delta",out var value)||value.ValueKind!=JsonValueKind.Object)return done;
            var content=ReadString(value,"content");var reasoning=ReadString(value,"reasoning_content");var cumulative=false;
            if(reasoning.Length==0)reasoning=ReadString(value,"thinking_content");
            if(reasoning.Length==0){reasoning=ReadReasoningDetails(value);cumulative=reasoning.Length>0;}
            delta=new(content,reasoning,cumulative);return true;
        }
        catch(JsonException){return false;}
        catch(KeyNotFoundException){return false;}
        catch(InvalidOperationException){return false;}
    }

    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;
    internal static string ReadReasoningDetails(JsonElement value)
    {
        if(!value.TryGetProperty("reasoning_details",out var details))return string.Empty;
        if(details.ValueKind==JsonValueKind.String)return details.GetString()??string.Empty;
        if(details.ValueKind!=JsonValueKind.Array)return string.Empty;
        return string.Concat(details.EnumerateArray().Select(item=>item.ValueKind==JsonValueKind.Object?ReadString(item,"text"):string.Empty));
    }
}
