#define MyAppName "MewuAI"
#define MyAppVersion "0.2.0"
#define MyAppPublisher "abnste"
#define MyAppURL "https://github.com/abnste/mewu_ai"
#define PublishDir "..\artifacts\release\win-x64"

[Setup]
AppId={{D9760D1F-112A-4DC7-97F4-8F2D905C1A36}
AppName={cm:AppDisplayName}
AppVersion={#MyAppVersion}
AppVerName={cm:AppDisplayName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\MewuAI
DefaultGroupName={cm:AppDisplayName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\artifacts\installer
OutputBaseFilename=MewuAI-Setup-{#MyAppVersion}-win-x64
SetupIconFile=..\Assets\MewuAI.ico
UninstallDisplayIcon={app}\MewuAI.exe
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
WizardSizePercent=110
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage
UsePreviousLanguage=no
CloseApplications=yes
RestartApplications=no
UsePreviousAppDir=yes
SetupLogging=yes
VersionInfoVersion=0.2.0.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription=MewuAI Windows installer
VersionInfoProductName=MewuAI
VersionInfoProductVersion={#MyAppVersion}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[CustomMessages]
english.AppDisplayName=MewuAI
chinesesimplified.AppDisplayName=喵呜AI

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PublishDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{cm:AppDisplayName}"; Filename: "{app}\MewuAI.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\{cm:AppDisplayName}"; Filename: "{app}\MewuAI.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\MewuAI.exe"; Description: "{cm:LaunchProgram,MewuAI}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\MewuAI.exe"; Flags: nowait skipifnotsilent
