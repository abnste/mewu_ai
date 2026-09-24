// SPDX-FileCopyrightText: 2026 shuziyuxingxing-stack, Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

// Based on shuziyuxingxing-stack's PR #10, item 2.
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

    // Validate before removing a pending RPC: malformed errors must still wake its waiter.
    internal static void Rpc(JsonElement value)
    {
        Object(value,"rpc");
        if(value.TryGetProperty("method",out var method))String(method,"rpc.method");
        if(value.TryGetProperty("error",out var error))
        {
            Object(error,"rpc.error");
            if(error.TryGetProperty("code",out var code)&&(code.ValueKind!=JsonValueKind.Number||!code.TryGetInt32(out _)))
                throw Invalid("rpc.error.code","integer");
        }
    }
}
