// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Text;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal static class AnnotationStreamPhase
{
    internal static bool HasCompletedAnswer(string value)
    {
        if(value.Length>2_000_000)return false;
        var text=value.AsSpan();
        while(text.Length>0&&char.IsWhiteSpace(text[0]))text=text[1..];
        if(!text.StartsWith("{"))return false;
        var bytes=Encoding.UTF8.GetBytes(text.ToString());
        try
        {
            var reader=new Utf8JsonReader(bytes,false,default);
            while(reader.Read())
                if(reader.TokenType==JsonTokenType.PropertyName&&reader.CurrentDepth==1&&reader.ValueTextEquals("answer"))
                    return reader.Read()&&reader.TokenType==JsonTokenType.String&&reader.ValueSpan.Length>0;
        }
        catch(JsonException){}
        finally{System.Security.Cryptography.CryptographicOperations.ZeroMemory(bytes);}
        return false;
    }
}
