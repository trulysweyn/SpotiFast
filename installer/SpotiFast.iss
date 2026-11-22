#ifndef MyAppVersion
#define MyAppVersion "0.5.6"
#endif

#define MyAppName "SpotiFast"
#define MyAppPublisher "trulysweyn"
#define MyAppExeName "SpotiFast.exe"

[Setup]
AppId={{4DA76719-15AD-4502-B3EF-0AD4502D4C2A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\SpotiFast
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UsePreviousAppDir=yes
SetupIconFile=..\assets\spotify.ico
UninstallDisplayIcon={app}\SpotiFast.exe
OutputDir=..\dist
OutputBaseFilename=SpotiFast-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=SpotiFast.exe

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\spotifast.exe"; DestDir: "{app}"; DestName: "SpotiFast.exe"; Flags: ignoreversion
Source: "..\target\release\spotifast_updater.exe"; DestDir: "{app}"; DestName: "SpotiFastUpdater.exe"; Flags: ignoreversion
Source: "..\dist\tools\yt-dlp\yt-dlp.exe"; DestDir: "{app}\tools\yt-dlp"; Flags: ignoreversion
Source: "..\dist\tools\mpv\*"; DestDir: "{app}\tools\mpv"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\SpotiFast"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\SpotiFast"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch SpotiFast"; Flags: postinstall nowait skipifsilent
