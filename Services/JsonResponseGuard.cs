using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal static class JsonResponseGuard
{
    internal static JsonElement Object(JsonElement value,string path)
        => value.ValueKind==JsonValueKind.Object?value:throw Invalid(path,"object");
    internal static JsonElement Required(JsonElement value,string property,string path)
    {
        Object(value,path);
        return value.TryGetProperty(property,out var result)?result:throw new InvalidDataException($"响应缺少字段 {path}.{property}。");
    }
    internal static JsonElement Array(JsonElement value,string path)
        => value.ValueKind==JsonValueKind.Array?value:throw Invalid(path,"array");
    internal static string String(JsonElement value,string path)
        => value.ValueKind==JsonValueKind.String?value.GetString()??string.Empty:throw Invalid(path,"string");
    private static InvalidDataException Invalid(string path,string expected)=>new($"响应字段 {path} 类型无效，预期 {expected}。");
}
