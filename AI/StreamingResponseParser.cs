using System.Text.Json;

namespace mewu_ai_Assistant.AI;

public static class StreamingResponseParser
{
    public static bool TryParse(string line,out string delta,out bool done)
    {
        delta=string.Empty;done=false;if(!line.StartsWith("data:",StringComparison.OrdinalIgnoreCase))return false;var payload=line[5..].Trim();if(payload=="[DONE]"){done=true;return true;}try{using var document=JsonDocument.Parse(payload);var choices=document.RootElement.GetProperty("choices");if(choices.GetArrayLength()==0)return true;var value=choices[0].GetProperty("delta");if(value.TryGetProperty("content",out var content)&&content.ValueKind==JsonValueKind.String)delta=content.GetString()??string.Empty;return true;}catch(JsonException){return false;}catch(KeyNotFoundException){return false;}catch(InvalidOperationException){return false;}
    }
}
