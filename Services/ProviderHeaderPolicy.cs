namespace mewu_ai_Assistant.Services;

public static class ProviderHeaderPolicy
{
    public static void EnsureValid(IReadOnlyDictionary<string,string> headers)
    {
        ArgumentNullException.ThrowIfNull(headers);
        var names=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach(var header in headers)
        {
            EnsureHeaderName(header.Key);
            if(!names.Add(header.Key))throw new InvalidOperationException($"Custom Header 名称不能忽略大小写后重复：{header.Key}");
            if(header.Value is null)throw new InvalidOperationException($"Custom Header {header.Key} 的值不能为 null");
            if(ProviderHeaderCredentialService.IsSensitive(header.Key)&&string.IsNullOrWhiteSpace(header.Value))
                throw new InvalidOperationException($"认证 Header {header.Key} 的值不能为空");
            if(header.Key.Contains('\r')||header.Key.Contains('\n')||header.Value.Contains('\r')||header.Value.Contains('\n'))
                throw new InvalidOperationException("Custom Header 不能包含换行符");
            EnsureNotTransportHeader(header.Key);
        }
    }

    public static void EnsureSafeToPersist(IReadOnlyDictionary<string,string> headers)
    {
        EnsureValid(headers);
        var sensitive=headers.Keys.FirstOrDefault(ProviderHeaderCredentialService.IsSensitive);
        if(sensitive is not null)throw new InvalidOperationException($"{sensitive} 仍含待加密的认证信息，拒绝明文保存 Provider 配置");
    }

    public static void EnsureCredentialMappingsValid(
        IReadOnlyDictionary<string,string> customHeaders,
        IReadOnlyDictionary<string,string> credentialMappings)
    {
        ArgumentNullException.ThrowIfNull(customHeaders);ArgumentNullException.ThrowIfNull(credentialMappings);
        EnsureValid(customHeaders);
        var customNames=customHeaders.Keys.ToHashSet(StringComparer.OrdinalIgnoreCase);
        var mappingNames=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach(var mapping in credentialMappings)
        {
            EnsureHeaderName(mapping.Key);EnsureNotTransportHeader(mapping.Key);
            if(!mappingNames.Add(mapping.Key))throw new InvalidOperationException($"敏感 Header 凭据映射不能忽略大小写后重复：{mapping.Key}");
            if(!ProviderHeaderCredentialService.IsSensitive(mapping.Key))throw new InvalidOperationException($"{mapping.Key} 不是敏感 Header，拒绝从凭据存储加载并发送");
            if(customNames.Contains(mapping.Key))throw new InvalidOperationException($"{mapping.Key} 不能同时出现在明文 Header 与敏感凭据映射中");
            if(string.IsNullOrWhiteSpace(mapping.Value)||mapping.Value.Length>128||mapping.Value.Any(character=>!char.IsLetterOrDigit(character)&&character is not '-' and not '_'))
                throw new InvalidOperationException($"{mapping.Key} 的凭据标识无效");
        }
    }

    private static void EnsureHeaderName(string name)
    {
        if(string.IsNullOrWhiteSpace(name))throw new InvalidOperationException("Custom Header 名称不能为空");
        if(!name.All(IsTokenCharacter))throw new InvalidOperationException($"Custom Header 名称无效：{name}");
    }

    private static void EnsureNotTransportHeader(string name)
    {
        if(name.Equals("Host",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Content-Length",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Transfer-Encoding",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Connection",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Trailer",StringComparison.OrdinalIgnoreCase)
            || name.Equals("Upgrade",StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException($"Custom Header 不允许覆盖传输层字段：{name}");
    }

    private static bool IsTokenCharacter(char value)=>char.IsAsciiLetterOrDigit(value)||value is '!' or '#' or '$' or '%' or '&' or '\'' or '*' or '+' or '-' or '.' or '^' or '_' or '`' or '|' or '~';
}
