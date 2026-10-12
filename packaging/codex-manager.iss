#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef AppId
  #define AppId "Contentrium.CodexManager"
#endif
#ifndef AppTitle
  #define AppTitle "Codex Manager"
#endif

[Setup]
AppId={#AppId}
AppName={#AppTitle}
AppVersion={#AppVersion}
AppPublisher=Contentrium
AppPublisherURL=https://github.com/contentriumkorea/codex-manager
AppSupportURL=https://github.com/contentriumkorea/codex-manager/issues
AppUpdatesURL=https://github.com/contentriumkorea/codex-manager/releases/latest
DefaultDirName={localappdata}\Programs\{#AppTitle}
DefaultGroupName={#AppTitle}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Codex-Manager-Setup
SetupIconFile={#SourceDir}\_internal\project_manager\ui\app-icon.ico
UninstallDisplayIcon={app}\_internal\project_manager\ui\app-icon.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
Uninstallable=yes
UsePreviousAppDir=yes
SetupLogging=yes

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "바탕 화면에 바로 가기 만들기"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\{#AppTitle}"; Filename: "{app}\CodexManager.exe"; WorkingDir: "{app}"; IconFilename: "{app}\_internal\project_manager\ui\app-icon.ico"; AppUserModelID: "Contentrium.CodexManager"
Name: "{userdesktop}\{#AppTitle}"; Filename: "{app}\CodexManager.exe"; WorkingDir: "{app}"; IconFilename: "{app}\_internal\project_manager\ui\app-icon.ico"; AppUserModelID: "Contentrium.CodexManager"; Tasks: desktopicon

[UninstallDelete]
; Only the application-owned runtime: future in-app updates can add new libraries.
Type: filesandordirs; Name: "{app}\_internal"

[Run]
Filename: "{app}\CodexManager.exe"; Description: "Codex Manager 실행"; Flags: nowait postinstall skipifsilent
