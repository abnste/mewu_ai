using mewu_ai_Assistant.Services;
using Xunit;

namespace MewuAI.Tests;

public sealed class HermesDiscoveryServiceTests
{
    [Fact]
    public void DiscoverAcceptsTheWellKnownFixedLocalInstallation()
    {
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(HermesDiscoveryService.WellKnownHome);
        var service = CreateService(fileSystem);

        var installation = Assert.IsType<mewu_ai_Assistant.Models.HermesInstallation>(service.Discover());

        Assert.Equal(HermesDiscoveryService.WellKnownHome, installation.HomePath);
        Assert.Equal(Path.Combine(installation.HomePath, "bin", "hermes.exe"), installation.ExecutablePath);
        Assert.Equal(Path.Combine(installation.HomePath, "hermes-agent"), installation.AgentPath);
        Assert.Equal(Path.Combine(installation.HomePath, "config.yaml"), installation.ConfigPath);
    }

    [Fact]
    public void DiscoverAcceptsOnlyTheSupportedHermesBinLayoutFromPath()
    {
        const string home = @"D:\Apps\Hermes";
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(home);
        var service = CreateService(
            fileSystem,
            (name, target) => name == "PATH" && target == EnvironmentVariableTarget.Process
                ? Path.Combine(home, "bin")
                : null);

        var installation = Assert.IsType<mewu_ai_Assistant.Models.HermesInstallation>(service.Discover());

        Assert.Equal(home, installation.HomePath);
    }

    [Fact]
    public void DiscoverAcceptsOfficialWindowsVenvLauncher()
    {
        const string home = @"D:\Users\Mewu\AppData\Local\hermes";
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(home, Path.Combine("hermes-agent", "venv", "Scripts", "hermes.exe"));
        var service = CreateService(
            fileSystem,
            (name, target) => name == "PATH" && target == EnvironmentVariableTarget.User
                ? Path.Combine(home, "hermes-agent", "venv", "Scripts")
                : null);

        var installation = Assert.IsType<mewu_ai_Assistant.Models.HermesInstallation>(service.Discover());

        Assert.Equal(home, installation.HomePath);
        Assert.Equal(Path.Combine(home, "hermes-agent", "venv", "Scripts", "hermes.exe"), installation.ExecutablePath);
    }

    [Fact]
    public void DiscoverAcceptsOfficialWindowsAgentBinLauncher()
    {
        const string home = @"D:\Users\Mewu\AppData\Local\hermes";
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(home, Path.Combine("hermes-agent", "bin", "hermes.exe"));
        var service = CreateService(
            fileSystem,
            (name, target) => name == "PATH" && target == EnvironmentVariableTarget.User
                ? Path.Combine(home, "hermes-agent", "bin")
                : null);

        var installation = Assert.IsType<mewu_ai_Assistant.Models.HermesInstallation>(service.Discover());

        Assert.Equal(home, installation.HomePath);
        Assert.Equal(Path.Combine(home, "hermes-agent", "bin", "hermes.exe"), installation.ExecutablePath);
    }

    [Fact]
    public void ValidateRejectsUncAndDevicePathsBeforeAnyFileSystemProbe()
    {
        var fileSystem = new FakeHermesDiscoveryFileSystem();

        Assert.Null(HermesDiscoveryService.Validate(@"\\server\share\Hermes", fileSystem));
        Assert.Null(HermesDiscoveryService.Validate(@"\\?\C:\Hermes", fileSystem));
        Assert.Empty(fileSystem.DriveTypeQueries);
        Assert.Empty(fileSystem.AttributeQueries);
    }

    [Fact]
    public void ValidateRejectsMappedNetworkDrivesBeforeReadingInstallationFiles()
    {
        const string home = @"Z:\Hermes";
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(home);
        fileSystem.SetDriveType(home, DriveType.Network);

        Assert.Null(HermesDiscoveryService.Validate(home, fileSystem));
        Assert.Empty(fileSystem.AttributeQueries);
    }

    [Theory]
    [InlineData(@"C:\Hermes\bin")]
    [InlineData(@"C:\Hermes\bin\hermes.exe")]
    public void ValidateRejectsReparsePointsOnAParentOrTheExecutable(string reparsePath)
    {
        const string home = @"C:\Hermes";
        var fileSystem = new FakeHermesDiscoveryFileSystem();
        fileSystem.AddInstallation(home);
        fileSystem.MarkReparsePoint(reparsePath);

        Assert.Null(HermesDiscoveryService.Validate(home, fileSystem));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("relative\\Hermes")]
    [InlineData("C:relative\\Hermes")]
    public void ValidateRejectsEmptyAndRelativeHomes(string? home)
    {
        var fileSystem = new FakeHermesDiscoveryFileSystem();

        Assert.Null(HermesDiscoveryService.Validate(home, fileSystem));
        Assert.Empty(fileSystem.DriveTypeQueries);
        Assert.Empty(fileSystem.AttributeQueries);
    }

    [Fact]
    public void CandidateHomesIgnoreEmptyRelativeDuplicateAndExecutableLikePathEntries()
    {
        var pathValue = string.Join(
            Path.PathSeparator,
            string.Empty,
            @"relative\bin",
            @"C:\Hermes\bin",
            @"C:\Hermes\bin\",
            @"C:\Other\bin\hermes.cmd");
        var service = CreateService(
            new FakeHermesDiscoveryFileSystem(),
            (name, target) => (name, target) switch
            {
                ("HERMES_HOME", EnvironmentVariableTarget.Process) => @"relative\Hermes",
                ("HERMES_HOME", EnvironmentVariableTarget.Machine) => @"C:\Hermes\",
                ("PATH", EnvironmentVariableTarget.Process) => pathValue,
                ("PATH", EnvironmentVariableTarget.User) => string.Empty,
                _ => null
            });

        var candidates = service.CandidateHomes().ToArray();

        Assert.Equal(new[] { HermesDiscoveryService.WellKnownHome }, candidates);
    }

    private static HermesDiscoveryService CreateService(
        FakeHermesDiscoveryFileSystem fileSystem,
        Func<string, EnvironmentVariableTarget, string?>? readEnvironmentVariable = null)
        => new(
            readEnvironmentVariable ?? (static (_, _) => null),
            static _ => string.Empty,
            fileSystem);

    private sealed class FakeHermesDiscoveryFileSystem : IHermesDiscoveryFileSystem
    {
        private readonly Dictionary<string, FileAttributes> _attributes = new(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, DriveType> _driveTypes = new(StringComparer.OrdinalIgnoreCase);

        internal List<string> AttributeQueries { get; } = [];
        internal List<string> DriveTypeQueries { get; } = [];

        internal void AddInstallation(string home, string? executableRelativePath = null)
        {
            AddDirectory(home);
            AddDirectory(Path.Combine(home, "bin"));
            AddDirectory(Path.Combine(home, "hermes-agent"));
            AddDirectory(Path.Combine(home, "hermes-agent", "hermes_cli"));
            AddFile(Path.Combine(home, executableRelativePath ?? Path.Combine("bin", "hermes.exe")));
            AddFile(Path.Combine(home, "config.yaml"));
            AddFile(Path.Combine(home, "hermes-agent", "hermes_cli", "main.py"));
            SetDriveType(home, DriveType.Fixed);
        }

        internal void MarkReparsePoint(string path)
        {
            var normalized = Normalize(path);
            _attributes[normalized] = _attributes.GetValueOrDefault(normalized) | FileAttributes.ReparsePoint;
        }

        internal void SetDriveType(string path, DriveType driveType)
        {
            var root = Path.GetPathRoot(Path.GetFullPath(path)) ?? throw new ArgumentException("Path has no root.", nameof(path));
            _driveTypes[root] = driveType;
        }

        public bool TryGetAttributes(string path, out FileAttributes attributes)
        {
            var normalized = Normalize(path);
            AttributeQueries.Add(normalized);
            return _attributes.TryGetValue(normalized, out attributes);
        }

        public bool TryGetDriveType(string rootPath, out DriveType driveType)
        {
            var normalized = Path.GetPathRoot(Path.GetFullPath(rootPath)) ?? rootPath;
            DriveTypeQueries.Add(normalized);
            return _driveTypes.TryGetValue(normalized, out driveType);
        }

        private void AddDirectory(string path)
        {
            var current = Normalize(path);
            var root = Path.GetPathRoot(current) ?? throw new ArgumentException("Path has no root.", nameof(path));
            while (true)
            {
                _attributes[current] = _attributes.GetValueOrDefault(current) | FileAttributes.Directory;
                if (string.Equals(current, root, StringComparison.OrdinalIgnoreCase))
                {
                    return;
                }

                current = Normalize(Path.GetDirectoryName(current) ?? root);
            }
        }

        private void AddFile(string path)
        {
            var normalized = Normalize(path);
            AddDirectory(Path.GetDirectoryName(normalized) ?? throw new ArgumentException("File has no directory.", nameof(path)));
            _attributes[normalized] = FileAttributes.Normal;
        }

        private static string Normalize(string path)
            => Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
    }
}
