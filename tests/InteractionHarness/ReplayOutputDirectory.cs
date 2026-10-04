// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
using System.Globalization;
using System.IO;
using System.Text;

/// <summary>Output isolation for opt-in replays; never changes product configuration.</summary>
internal static class ReplayOutputDirectory
{
    private const string InheritedRootVariable = "MEWU_REPLAY_TEMP_ROOT";
    private const string Marker = ".mewu-replay";

    internal static string PrepareWorkingDirectory(string scenario)
    {
        if (string.IsNullOrEmpty(scenario) || scenario.Any(character => !char.IsAsciiLetterOrDigit(character) && character != '-'))
            throw new ArgumentException("Use a simple replay scenario name.", nameof(scenario));
        var directory = Normalize(Environment.CurrentDirectory);
        var root = FindMarkedRoot(directory);
        if (root is null)
        {
            root = Normalize(Path.Combine(Path.GetTempPath(), "MewuAI-Replays"));
            EnsureSafePath(root);
            directory = Path.Combine(root, DateTime.UtcNow.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture) +
                "-" + scenario + "-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, Marker), "Synthetic MewuAI replay output\n", new UTF8Encoding(false));
        }
        EnsureSafePath(directory);
        // Workers change TEMP/TMP to their own scenario directory. Carry the
        // originally approved root separately so they reuse the parent's tree.
        Environment.SetEnvironmentVariable(InheritedRootVariable, root);
        Environment.CurrentDirectory = directory;
        Console.WriteLine("Replay output: " + directory);
        return directory;
    }

    internal static string ValidateDirectory(string path)
    {
        var directory = Normalize(path);
        if (FindMarkedRoot(directory) is null)
            throw new InvalidOperationException("Replay output must remain inside its marked temporary task directory.");
        EnsureSafePath(directory);
        return directory;
    }

    private static string? FindMarkedRoot(string directory)
    {
        foreach (var root in TemporaryRoots())
        {
            if (!IsWithin(directory, root)) continue;
            EnsureSafePath(directory);
            for (var current = new DirectoryInfo(directory); current is not null && IsWithin(current.FullName, root, allowRoot: true); current = current.Parent)
            {
                foreach (var name in new[] { Marker, ".mini-temp" })
                {
                    var marker = Path.Combine(current.FullName, name);
                    if (!File.Exists(marker)) continue;
                    if (File.GetAttributes(marker).HasFlag(FileAttributes.ReparsePoint))
                        throw new InvalidOperationException("Replay markers cannot be filesystem links.");
                    return root;
                }
            }
        }
        return null;
    }

    private static IEnumerable<string> TemporaryRoots()
    {
        var roots = new[]
        {
            Environment.GetEnvironmentVariable(InheritedRootVariable),
            Environment.GetEnvironmentVariable("MINI_TEMP_ROOT"),
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Temp", "Mini"),
            Path.Combine(Path.GetTempPath(), "MewuAI-Replays")
        };
        return roots.Where(root => !string.IsNullOrWhiteSpace(root)).Select(root => Normalize(root!))
            .Distinct(StringComparer.OrdinalIgnoreCase);
    }

    private static void EnsureSafePath(string path)
    {
        var userData = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "MewuAI");
        var installedProduct = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs", "MewuAI");
        if (IsWithin(path, userData, allowRoot: true) || IsWithin(path, installedProduct, allowRoot: true))
            throw new InvalidOperationException("Replay output cannot use the installed application or its user data directory.");
        for (var current = new DirectoryInfo(path); current is not null; current = current.Parent)
        {
            if (current.Exists && current.Attributes.HasFlag(FileAttributes.ReparsePoint))
                throw new InvalidOperationException("Replay output directories cannot use filesystem links.");
            if (Directory.Exists(Path.Combine(current.FullName, ".git")) || File.Exists(Path.Combine(current.FullName, ".git")) ||
                File.Exists(Path.Combine(current.FullName, "mewu_ai_Assistant.csproj")))
                throw new InvalidOperationException("Replay output cannot be created inside a source checkout.");
        }
    }

    private static string Normalize(string path) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
    private static bool IsWithin(string path, string root, bool allowRoot = false)
    {
        path = Normalize(path); root = Normalize(root);
        return (allowRoot && string.Equals(path, root, StringComparison.OrdinalIgnoreCase)) ||
            path.StartsWith(Path.EndsInDirectorySeparator(root) ? root : root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
    }
}
