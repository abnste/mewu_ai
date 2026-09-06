// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json;

namespace mewu_ai_Assistant.Services;

internal sealed record MiniMaxCodeDesktopSession(string BaseUrl,string AccessToken);
internal sealed record MiniMaxCodeModel(string Model,string Name)
{
    internal bool SupportsVision=>Model.Contains("MiniMax-M3",StringComparison.OrdinalIgnoreCase);
    public override string ToString()=>Name;
}

/// <summary>Discovers the official MiniMax Code desktop session without using the CLI.</summary>
internal static class MiniMaxCodeRuntime
{
    internal const string DefaultBaseUrl="https://agent.minimax.cn/mavis/api/v1/llm/v1";
    internal static readonly IReadOnlyList<MiniMaxCodeModel> KnownModels=[
        new("minimax/MiniMax-M3","MiniMax-M3（支持图片和视频）"),
        new("minimax/MiniMax-M2.7","MiniMax-M2.7（文字）"),
        new("minimax/MiniMax-M2.7-highspeed","MiniMax-M2.7-highspeed（文字）")];

    internal static MiniMaxCodeDesktopSession? TryGetDesktopSession()
    {
        var path=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),"MiniMax","minimax-agent-cn-config.json");
        byte[]? fileBytes=null;
        try
        {
            if(!File.Exists(path))return null;
            fileBytes=File.ReadAllBytes(path);
            using var document=JsonDocument.Parse(fileBytes);
            if(!document.RootElement.TryGetProperty("tokens",out var tokens)||tokens.ValueKind!=JsonValueKind.Object||!tokens.TryGetProperty("accessToken",out var access)||access.ValueKind!=JsonValueKind.String)return null;
            var token=access.GetString()?.Trim();
            return string.IsNullOrWhiteSpace(token)||token.Length>16_384?null:new(DefaultBaseUrl,token);
        }
        catch(Exception ex)when(ex is IOException or UnauthorizedAccessException or JsonException or NotSupportedException){return null;}
        finally{if(fileBytes is not null)CryptographicOperations.ZeroMemory(fileBytes);}
    }

    internal static bool LaunchDesktop()
    {
        var path=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"Programs","MiniMax Code","MiniMax Code.exe");
        if(!File.Exists(path))return false;
        try{Process.Start(new ProcessStartInfo(path){UseShellExecute=true});return true;}
        catch(Exception ex)when(ex is InvalidOperationException or System.ComponentModel.Win32Exception){return false;}
    }

    internal static void ValidateModel(string model)
    {
        if(string.IsNullOrWhiteSpace(model)||model.Length>300||model.Any(char.IsControl)||model.Any(char.IsWhiteSpace)||!KnownModels.Any(item=>item.Model.Equals(model,StringComparison.OrdinalIgnoreCase)))
            throw new InvalidOperationException("请在 MiniMax Code 页选择可用模型。");
    }
}
