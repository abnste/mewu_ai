using System.Security;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

public sealed class HermesDiscoveryService
{
    internal const string WellKnownHome = @"C:\Hermes";
    internal const int MaxPathEntriesPerScope = 128;
    private const int MaxCandidateCharacters = 32767;
    private const int MaxPathComponents = 256;
    private readonly Func<string, EnvironmentVariableTarget, string?> _readEnvironmentVariable;
    private readonly Func<Environment.SpecialFolder, string> _getFolderPath;
    private readonly IHermesDiscoveryFileSystem _fileSystem;

    internal static readonly string[] RequiredAgentFiles =
    [
        Path.Combine("hermes_cli", "main.py")
    ];

    public HermesDiscoveryService()
        : this(
            static (name, target) => Environment.GetEnvironmentVariable(name, target),
            static folder => Environment.GetFolderPath(folder),
            SystemHermesDiscoveryFileSystem.Instance)
    {
    }

    internal HermesDiscoveryService(
        Func<string, EnvironmentVariableTarget, string?> readEnvironmentVariable,
        Func<Environment.SpecialFolder, string> getFolderPath,
        IHermesDiscoveryFileSystem fileSystem)
    {
        _readEnvironmentVariable = readEnvironmentVariable ?? throw new ArgumentNullException(nameof(readEnvironmentVariable));
        _getFolderPath = getFolderPath ?? throw new ArgumentNullException(nameof(getFolderPath));
        _fileSystem = fileSystem ?? throw new ArgumentNullException(nameof(fileSystem));
    }

    public HermesInstallation? Discover()
    {
        foreach (var candidate in CandidateHomes())
        {
            var installation = Validate(candidate, _fileSystem);
            if (installation is not null)
            {
                return installation;
            }
        }

        return null;
    }

    internal IEnumerable<string> CandidateHomes()
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        foreach (var value in ConfiguredHomes())
        {
            if (TryNormalizeLocalAbsolutePath(value, out var normalized) && seen.Add(normalized))
            {
                yield return normalized;
            }
        }

        foreach (var target in EnvironmentTargets)
        {
            var pathValue = TryReadEnvironmentVariable("PATH", target);
            if (string.IsNullOrWhiteSpace(pathValue))
            {
                continue;
            }

            var entries = pathValue.Split(
                [Path.PathSeparator],
                MaxPathEntriesPerScope + 1,
                StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            var entryCount = Math.Min(entries.Length, MaxPathEntriesPerScope);
            for (var index = 0; index < entryCount; index++)
            {
                if (!TryNormalizeLocalAbsolutePath(entries[index], out var pathDirectory) ||
                    !string.Equals(Path.GetFileName(pathDirectory), "bin", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var home = Path.GetDirectoryName(pathDirectory);
                if (TryNormalizeLocalAbsolutePath(home, out var normalized) && seen.Add(normalized))
                {
                    // PATH discovery deliberately understands only the supported
                    // <home>\bin\hermes.exe layout. It never searches a drive,
                    // invokes command shims, or honors PATHEXT aliases.
                    yield return normalized;
                }
            }
        }
    }

    internal static HermesInstallation? Validate(string? home)
        => Validate(home, SystemHermesDiscoveryFileSystem.Instance);

    internal static HermesInstallation? Validate(string? home, IHermesDiscoveryFileSystem fileSystem)
    {
        ArgumentNullException.ThrowIfNull(fileSystem);
        if (!TryNormalizeLocalAbsolutePath(home, out var normalized) ||
            !TryGetDriveLetterRoot(normalized, out var driveRoot) ||
            !fileSystem.TryGetDriveType(driveRoot, out var driveType) ||
            driveType != DriveType.Fixed)
        {
            return null;
        }

        var agentPath = Path.Combine(normalized, "hermes-agent");
        var executable = Path.Combine(normalized, "bin", "hermes.exe");
        var config = Path.Combine(normalized, "config.yaml");
        if (!IsTrustedExistingPath(normalized, driveRoot, expectDirectory: true, fileSystem) ||
            !IsTrustedExistingPath(agentPath, driveRoot, expectDirectory: true, fileSystem) ||
            !IsTrustedExistingPath(executable, driveRoot, expectDirectory: false, fileSystem) ||
            !IsTrustedExistingPath(config, driveRoot, expectDirectory: false, fileSystem))
        {
            return null;
        }

        foreach (var relative in RequiredAgentFiles)
        {
            if (!IsTrustedExistingPath(
                    Path.Combine(agentPath, relative),
                    driveRoot,
                    expectDirectory: false,
                    fileSystem))
            {
                return null;
            }
        }

        return new HermesInstallation(normalized, agentPath, executable, config);
    }

    private static readonly EnvironmentVariableTarget[] EnvironmentTargets =
    [
        EnvironmentVariableTarget.Process,
        EnvironmentVariableTarget.User,
        EnvironmentVariableTarget.Machine
    ];

    private IEnumerable<string?> ConfiguredHomes()
    {
        foreach (var target in EnvironmentTargets)
        {
            yield return TryReadEnvironmentVariable("HERMES_HOME", target);
        }

        yield return WellKnownHome;

        var localApplicationData = TryGetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (!string.IsNullOrWhiteSpace(localApplicationData))
        {
            yield return Path.Combine(localApplicationData, "hermes");
        }

        var userProfile = TryGetFolderPath(Environment.SpecialFolder.UserProfile);
        if (!string.IsNullOrWhiteSpace(userProfile))
        {
            yield return Path.Combine(userProfile, ".hermes");
        }
    }

    private string? TryReadEnvironmentVariable(string name, EnvironmentVariableTarget target)
    {
        try
        {
            return _readEnvironmentVariable(name, target);
        }
        catch (Exception ex) when (ex is SecurityException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    private string? TryGetFolderPath(Environment.SpecialFolder folder)
    {
        try
        {
            return _getFolderPath(folder);
        }
        catch (Exception ex) when (ex is SecurityException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    private static bool TryNormalizeLocalAbsolutePath(string? value, out string normalized)
    {
        normalized = string.Empty;
        if (string.IsNullOrWhiteSpace(value))
        {
            return false;
        }

        var candidate = value.Trim();
        if (candidate.Length >= 2 && candidate[0] == '"' && candidate[^1] == '"')
        {
            candidate = candidate[1..^1].Trim();
        }

        if (candidate.Length == 0 || candidate.Length > MaxCandidateCharacters || candidate.Contains('"'))
        {
            return false;
        }

        try
        {
            candidate = Environment.ExpandEnvironmentVariables(candidate);
            if (candidate.Length == 0 || candidate.Length > MaxCandidateCharacters ||
                !Path.IsPathFullyQualified(candidate))
            {
                return false;
            }

            var fullPath = Path.TrimEndingDirectorySeparator(Path.GetFullPath(candidate));
            if (!TryGetDriveLetterRoot(fullPath, out _))
            {
                return false;
            }

            normalized = fullPath;
            return true;
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException or SecurityException)
        {
            return false;
        }
    }

    private static bool TryGetDriveLetterRoot(string path, out string root)
    {
        root = Path.GetPathRoot(path) ?? string.Empty;
        return root.Length == 3 &&
               char.IsAsciiLetter(root[0]) &&
               root[1] == Path.VolumeSeparatorChar &&
               (root[2] == Path.DirectorySeparatorChar || root[2] == Path.AltDirectorySeparatorChar);
    }

    private static bool IsTrustedExistingPath(
        string path,
        string expectedDriveRoot,
        bool expectDirectory,
        IHermesDiscoveryFileSystem fileSystem)
    {
        if (path.Length > MaxCandidateCharacters ||
            !Path.IsPathFullyQualified(path) ||
            !TryGetDriveLetterRoot(path, out var actualDriveRoot) ||
            !string.Equals(expectedDriveRoot, actualDriveRoot, StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        var chain = new Stack<string>();
        var current = Path.TrimEndingDirectorySeparator(path);
        var reachedRoot = false;
        for (var componentCount = 0; componentCount < MaxPathComponents; componentCount++)
        {
            chain.Push(current);
            if (string.Equals(current, expectedDriveRoot, StringComparison.OrdinalIgnoreCase))
            {
                reachedRoot = true;
                break;
            }

            var parent = Path.GetDirectoryName(current);
            if (string.IsNullOrWhiteSpace(parent) || string.Equals(parent, current, StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            current = Path.TrimEndingDirectorySeparator(parent);
        }

        if (!reachedRoot)
        {
            return false;
        }

        while (chain.Count > 0)
        {
            var component = chain.Pop();
            var isLeaf = chain.Count == 0;
            if (!fileSystem.TryGetAttributes(component, out var attributes) ||
                attributes.HasFlag(FileAttributes.ReparsePoint))
            {
                return false;
            }

            var isDirectory = attributes.HasFlag(FileAttributes.Directory);
            if ((!isLeaf && !isDirectory) || (isLeaf && isDirectory != expectDirectory))
            {
                return false;
            }
        }

        return true;
    }
}

internal interface IHermesDiscoveryFileSystem
{
    bool TryGetAttributes(string path, out FileAttributes attributes);
    bool TryGetDriveType(string rootPath, out DriveType driveType);
}

internal sealed class SystemHermesDiscoveryFileSystem : IHermesDiscoveryFileSystem
{
    internal static SystemHermesDiscoveryFileSystem Instance { get; } = new();

    private SystemHermesDiscoveryFileSystem()
    {
    }

    public bool TryGetAttributes(string path, out FileAttributes attributes)
    {
        try
        {
            attributes = File.GetAttributes(path);
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or
                                   NotSupportedException or SecurityException)
        {
            attributes = default;
            return false;
        }
    }

    public bool TryGetDriveType(string rootPath, out DriveType driveType)
    {
        try
        {
            driveType = new DriveInfo(rootPath).DriveType;
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or SecurityException)
        {
            driveType = DriveType.Unknown;
            return false;
        }
    }
}
