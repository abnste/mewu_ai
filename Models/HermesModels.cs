namespace mewu_ai_Assistant.Models;

public sealed record HermesInstallation(
    string HomePath,
    string AgentPath,
    string ExecutablePath,
    string ConfigPath);

public sealed record HermesConnectionInfo(
    HermesInstallation Installation,
    int Port,
    Uri HttpBaseUri);

public sealed record HermesModelOption(
    string Provider,
    string Model,
    string DisplayName,
    IReadOnlyList<string> ReasoningEfforts,
    bool IsCurrent=false)
{
    public string Key => string.IsNullOrWhiteSpace(Provider) ? Model : $"{Provider}\u001f{Model}";
    public override string ToString()=>DisplayName;
}

public sealed record HermesRpcEvent(
    string Type,
    string SessionId,
    System.Text.Json.JsonElement Payload);

public enum HermesConversationKind { Text,Screen }
