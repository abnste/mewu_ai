using System.Diagnostics;
using mewu_ai_Assistant.Models;

namespace mewu_ai_Assistant.Services;

internal static class HermesLaunchPolicy
{
    internal static void Configure(ProcessStartInfo start, HermesInstallation installation)
    {
        var agent = Path.GetFullPath(installation.AgentPath);
        var scripts = Path.GetDirectoryName(Path.GetFullPath(installation.ExecutablePath));
        var environments = new[] { Path.Combine(agent, "venv"), Path.Combine(agent, ".venv") };
        // Never mix an interpreter from .venv with modules from venv (or vice versa).
        var selected = environments.FirstOrDefault(root => string.Equals(
            Path.Combine(root, "Scripts"), scripts, StringComparison.OrdinalIgnoreCase));
        var candidates = selected is null ? environments : new[] { selected };
        var python = candidates.Select(root => Path.Combine(root, "Scripts", "python.exe"))
            .FirstOrDefault(path => HermesDiscoveryService.IsTrustedRuntimePath(path, false));
        var venv = python is null ? null : Path.GetDirectoryName(Path.GetDirectoryName(python));

        start.Environment.Remove("PYTHONHOME");
        start.Environment.Remove("VIRTUAL_ENV");
        start.Environment.Remove("PYTHONPATH");
        start.Environment["PYTHONUTF8"] = "1";
        start.Environment["PYTHONIOENCODING"] = "utf-8";
        if (python is not null)
        {
            start.Environment["PYTHONNOUSERSITE"] = "1";
            start.FileName = python;
            start.ArgumentList.Add("-m");
            start.ArgumentList.Add("hermes_cli.main");
            start.Environment["VIRTUAL_ENV"] = venv!;
            // Match the official Desktop module launch, without inheriting another
            // application's Python search path or mounting a different venv.
            start.Environment["PYTHONPATH"] = agent;
        }

        var managed = new[]
        {
            venv is null ? scripts : Path.Combine(venv, "Scripts"),
            Path.Combine(installation.HomePath, "node"),
            Path.Combine(installation.HomePath, "node", "bin"),
            Path.Combine(installation.HomePath, "git", "cmd"),
            Path.Combine(installation.HomePath, "git", "usr", "bin")
        };
        var pathKey = start.Environment.Keys.FirstOrDefault(key => string.Equals(key, "PATH", StringComparison.OrdinalIgnoreCase)) ?? "PATH";
        start.Environment.TryGetValue(pathKey, out var inheritedPath);
        start.Environment[pathKey] = string.Join(Path.PathSeparator,
            managed.Where(path => path is not null && HermesDiscoveryService.IsTrustedRuntimePath(path, true))
                .Concat(new[] { inheritedPath }).Where(path => !string.IsNullOrWhiteSpace(path)));
    }
}
