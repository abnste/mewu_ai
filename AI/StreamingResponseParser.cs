using System.Text.Json;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.AI;

public static class StreamingResponseParser
{
    public static bool TryParse(string line,out AiStreamDelta delta,out bool done)
    {
        delta=new(string.Empty,string.Empty);done=false;if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))return false;var payload=line[5..].Trim();if(payload=="[DONE]"){done=true;return true;}try{using var document=JsonDocument.Parse(payload);var choices=document.RootElement.GetProperty("choices");if(choices.GetArrayLength()==0)return true;var value=choices[0].GetProperty("delta");var content=ReadString(value,"content");var reasoning=ReadString(value,"reasoning_content");if(reasoning.Length==0)reasoning=ReadString(value,"thinking_content");delta=new(content,reasoning);return true;}catch(JsonException){return false;}catch(KeyNotFoundException){return false;}catch(InvalidOperationException){return false;}
    }

    private static string ReadString(JsonElement value,string name)=>value.TryGetProperty(name,out var property)&&property.ValueKind==JsonValueKind.String?property.GetString()??string.Empty:string.Empty;
}
