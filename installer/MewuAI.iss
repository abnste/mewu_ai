; SPDX-License-Identifier: MPL-2.0
#define MyAppName "MewuAI"
#define MyAppVersion "0.2.6"
#define MyAppPublisher "abnste"
#define MyAppURL "https://github.com/abnste/mewu_ai"
#ifndef PublishDir
  #define PublishDir "..\artifacts\release\win-x64"
#endif

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
VersionInfoVersion=0.2.6.0
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
; On Windows 11 26100, direct children of Inno 6.7.1 inherit RedirectionGuard.
; uv-managed Python junctions then fail with error 448. Hand off to the user's
; desktop shell so the application gets normal desktop process state, while
; Setup keeps its own RedirectionGuard protection throughout installation.
Filename: "{win}\explorer.exe"; Parameters: """{app}\MewuAI.exe"""; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,MewuAI}"; Flags: nowait runasoriginaluser postinstall skipifsilent
Filename: "{win}\explorer.exe"; Parameters: """{app}\MewuAI.exe"""; WorkingDir: "{app}"; Flags: nowait runasoriginaluser skipifnotsilent
