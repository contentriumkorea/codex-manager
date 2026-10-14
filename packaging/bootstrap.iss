; Fixed online installer. Application versions live only in the release channel.
#ifdef QA
  #define AppId "Contentrium.CodexManager.QA"
  #define AppTitle "Codex Manager QA"
#else
  #ifdef ChannelBase
    #error Channel override is only allowed in a QA build
  #endif
  #define AppId "Contentrium.CodexManager"
  #define AppTitle "Codex Manager"
#endif
#ifndef ChannelBase
  #define ChannelBase "https://github.com/contentriumkorea/codex-manager"
#endif

[Setup]
AppId={#AppId}
AppName={#AppTitle}
AppVersion=1.0.0
AppVerName={#AppTitle}
AppPublisher=Contentrium
AppPublisherURL=https://github.com/contentriumkorea/codex-manager
AppSupportURL=https://github.com/contentriumkorea/codex-manager/issues
AppUpdatesURL=https://github.com/contentriumkorea/codex-manager/releases/latest
DefaultDirName={localappdata}\Programs\{#AppTitle}
DefaultGroupName={#AppTitle}
DisableProgramGroupPage=yes
DisableDirPage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Codex-Manager-Setup
SetupIconFile={#IconPath}
UninstallDisplayIcon={app}\_internal\project_manager\ui\app-icon.ico
Compression=lzma2
SolidCompression=yes
ArchiveExtraction=full
WizardStyle=modern
CloseApplications=no
RestartApplications=no
Uninstallable=yes
UsePreviousAppDir=yes
SetupLogging=yes
ExtraDiskSpaceRequired=10485760

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
korean.DesktopIcon=바탕 화면에 바로 가기 만들기
korean.Launch=Codex Manager 실행
korean.DownloadTitle=최신 Codex Manager 다운로드
korean.DownloadDescription=최신 버전을 확인하고 설치 파일을 안전하게 검증합니다. 인터넷 연결이 필요합니다.
korean.ExtractTitle=설치 준비
korean.ExtractDescription=검증된 파일의 압축을 풀고 있습니다.
korean.InvalidMetadata=최신 버전 정보를 확인할 수 없습니다. 잠시 후 다시 실행해 주세요.
korean.InvalidPayload=다운로드한 파일이 올바르지 않습니다. 잠시 후 다시 실행해 주세요.
korean.AppRunning=Codex Manager가 실행 중이거나 설치 폴더가 잠겨 있습니다. 진행 중인 작업을 마친 뒤 프로그램을 종료하고 다시 시도해 주세요.
korean.UnsafeTarget=비어 있는 새 폴더 또는 기존 Codex Manager 설치 폴더를 선택해 주세요. 연결 폴더에는 설치할 수 없습니다.
korean.NoSpace=다운로드와 설치에 필요한 여유 공간이 부족합니다.
korean.DownloadError=설치를 준비하지 못했습니다. 기존 프로그램은 변경하지 않았습니다. 인터넷 연결과 여유 공간을 확인한 뒤 다시 시도해 주세요.
english.DesktopIcon=Create a desktop shortcut
english.Launch=Launch Codex Manager
english.DownloadTitle=Download the latest Codex Manager
english.DownloadDescription=Checking the latest release and verifying the download. An internet connection is required.
english.ExtractTitle=Preparing installation
english.ExtractDescription=Extracting the verified application files.
english.InvalidMetadata=The latest release information is invalid. Please try again later.
english.InvalidPayload=The downloaded application is invalid. Please try again later.
english.AppRunning=Codex Manager is running or its installation folder is locked. Finish any active work, close the app and try again.
english.UnsafeTarget=Choose an empty folder or an existing Codex Manager installation. Linked folders are not supported.
english.NoSpace=There is not enough free space to download and install the application.
english.DownloadError=Unable to prepare installation. The existing application has not been changed. Check your internet connection and free disk space, then try again.

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; Flags: unchecked

[Files]
Source: "{tmp}\payload\CodexManager\*"; DestDir: "{app}"; Flags: external ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\{#AppTitle}"; Filename: "{app}\CodexManager.exe"; WorkingDir: "{app}"; IconFilename: "{app}\_internal\project_manager\ui\app-icon.ico"; AppUserModelID: "Contentrium.CodexManager"
Name: "{userdesktop}\{#AppTitle}"; Filename: "{app}\CodexManager.exe"; WorkingDir: "{app}"; IconFilename: "{app}\_internal\project_manager\ui\app-icon.ico"; AppUserModelID: "Contentrium.CodexManager"; Tasks: desktopicon

[UninstallDelete]
Type: filesandordirs; Name: "{app}\_internal"

[Run]
Filename: "{app}\CodexManager.exe"; Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[Code]
var
  DownloadPage: TDownloadWizardPage;
  ExtractionPage: TExtractionWizardPage;
  ApplicationVersion: String;
  Prepared: Boolean;

function GetFileAttributesW(Name: String): LongWord;
  external 'GetFileAttributesW@kernel32.dll stdcall';

function AllChars(const Value, Allowed: String): Boolean;
var I: Integer;
begin
  Result := Length(Value) > 0;
  for I := 1 to Length(Value) do
    if Pos(Value[I], Allowed) = 0 then Result := False;
end;

function ValidVersion(const Value: String): Boolean;
var Parts: TStringList; I: Integer;
begin
  Result := False;
  if (Length(Value) < 5) or (Length(Value) > 20) then Exit;
  Parts := TStringList.Create;
  try
    Parts.Delimiter := '.';
    Parts.StrictDelimiter := True;
    Parts.DelimitedText := Value;
    if Parts.Count <> 3 then Exit;
    for I := 0 to 2 do begin
      if not AllChars(Parts[I], '0123456789') then Exit;
      if Length(Parts[I]) > 6 then Exit;
      if (Length(Parts[I]) > 1) and (Parts[I][1] = '0') then Exit;
    end;
    Result := True;
  finally
    Parts.Free;
  end;
end;

function ReadSize(const Ini, Key: String): Int64;
var Value: String;
begin
  Value := GetIniString('release', Key, '', Ini);
  if not AllChars(Value, '0123456789') then
    RaiseException(CustomMessage('InvalidMetadata'));
  Result := StrToInt64Def(Value, -1);
  if (Result <= 0) or (Result > 2147483647) then
    RaiseException(CustomMessage('InvalidMetadata'));
end;

function SizeOfFile(const Name: String): Int64;
var Stream: TFileStream;
begin
  Stream := TFileStream.Create(Name, fmOpenRead or fmShareDenyWrite);
  try Result := Stream.Size; finally Stream.Free; end;
end;

procedure RejectLinks(const Path: String; const Recurse: Boolean);
var Attributes: LongWord; Entry: TFindRec;
begin
  Attributes := GetFileAttributesW(Path);
  if Attributes = $FFFFFFFF then Exit;
  if (Attributes and $400) <> 0 then RaiseException(CustomMessage('UnsafeTarget'));
  if Recurse and ((Attributes and FILE_ATTRIBUTE_DIRECTORY) <> 0) then
    if FindFirst(AddBackslash(Path) + '*', Entry) then
      try
        repeat
          if (Entry.Name <> '.') and (Entry.Name <> '..') then
            RejectLinks(AddBackslash(Path) + Entry.Name, True);
        until not FindNext(Entry);
      finally FindClose(Entry); end;
end;

procedure CheckTarget;
var Target, Parent, Executable: String; Entry: TFindRec; Stream: TFileStream;
begin
  Target := RemoveBackslashUnlessRoot(ExpandConstant('{app}'));
  if Length(Target) <= Length(AddBackslash(ExtractFileDrive(Target))) then
    RaiseException(CustomMessage('UnsafeTarget'));
  Parent := Target;
  repeat
    RejectLinks(Parent, False);
    Target := ExtractFileDir(Parent);
    if Target = Parent then Break;
    Parent := Target;
  until Length(Parent) <= 3;
  Target := ExpandConstant('{app}');
  RejectLinks(Target, True);
  Executable := AddBackslash(Target) + 'CodexManager.exe';
  if FileExists(Executable) and FileExists(AddBackslash(Target) + 'install-manifest.json') then begin
    try
      Stream := TFileStream.Create(Executable, fmOpenReadWrite or fmShareExclusive);
      Stream.Free;
    except RaiseException(CustomMessage('AppRunning')); end;
  end else if FindFirst(AddBackslash(Target) + '*', Entry) then
    try
      repeat
        if (Entry.Name <> '.') and (Entry.Name <> '..') then
          RaiseException(CustomMessage('UnsafeTarget'));
      until not FindNext(Entry);
    finally FindClose(Entry); end;
end;

procedure RequireSpace(const Path: String; const Needed: Int64);
var FreeBytes, TotalBytes: Int64;
begin
  if not GetSpaceOnDisk64(Path, FreeBytes, TotalBytes) or (FreeBytes < Needed) then
    RaiseException(CustomMessage('NoSpace'));
end;

procedure PreparePayload;
var Ini, Hash, Archive, Destination: String; ArchiveSize, ExpandedSize: Int64;
begin
  CheckTarget;
  Ini := ExpandConstant('{tmp}\install.ini');
  DownloadPage.Clear;
  DownloadPage.Add('{#ChannelBase}/releases/latest/download/install.ini', 'install.ini', '');
  DownloadPage.Show;
  try DownloadPage.Download; finally DownloadPage.Hide; end;
  if SizeOfFile(Ini) > 8192 then RaiseException(CustomMessage('InvalidMetadata'));
  if GetIniString('release', 'protocol', '', Ini) <> '1' then RaiseException(CustomMessage('InvalidMetadata'));
  ApplicationVersion := GetIniString('release', 'version', '', Ini);
  if not ValidVersion(ApplicationVersion) then RaiseException(CustomMessage('InvalidMetadata'));
  Hash := Lowercase(GetIniString('release', 'sha256', '', Ini));
  if (Length(Hash) <> 64) or not AllChars(Hash, '0123456789abcdef') then
    RaiseException(CustomMessage('InvalidMetadata'));
  ArchiveSize := ReadSize(Ini, 'size');
  ExpandedSize := ReadSize(Ini, 'expanded_size');
  RequireSpace(ExpandConstant('{tmp}'), ArchiveSize + 2 * ExpandedSize + 10485760);
  RequireSpace(ExtractFileDrive(ExpandConstant('{app}')), 2 * ExpandedSize + 10485760);
  Archive := ExpandConstant('{tmp}\Codex-Manager-Windows-x64.zip');
  DownloadPage.Clear;
  DownloadPage.Add('{#ChannelBase}/releases/download/v' + ApplicationVersion + '/Codex-Manager-Windows-x64.zip',
    'Codex-Manager-Windows-x64.zip', Hash);
  DownloadPage.Show;
  try DownloadPage.Download; finally DownloadPage.Hide; end;
  if SizeOfFile(Archive) <> ArchiveSize then RaiseException(CustomMessage('InvalidPayload'));
  Destination := ExpandConstant('{tmp}\payload');
  if DirExists(Destination) then DelTree(Destination, True, True, True);
  ExtractionPage.Clear;
  ExtractionPage.Add(Archive, Destination, True);
  ExtractionPage.Show;
  try ExtractionPage.Extract; finally ExtractionPage.Hide; end;
  if not FileExists(Destination + '\CodexManager\CodexManager.exe') or
     not FileExists(Destination + '\CodexManager\install-manifest.json') or
     not DirExists(Destination + '\CodexManager\_internal') then
    RaiseException(CustomMessage('InvalidPayload'));
  RejectLinks(Destination, True);
  Prepared := True;
  Log('Verified application version: ' + ApplicationVersion);
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(CustomMessage('DownloadTitle'), CustomMessage('DownloadDescription'), nil);
  DownloadPage.ShowBaseNameInsteadOfUrl := True;
  ExtractionPage := CreateExtractionPage(CustomMessage('ExtractTitle'), CustomMessage('ExtractDescription'), nil);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = wpReady then
    try
      Prepared := False;
      PreparePayload;
    except
      Log(GetExceptionMessage);
      SuppressibleMsgBox(CustomMessage('DownloadError') + #13#10#13#10 + GetExceptionMessage, mbCriticalError, MB_OK, IDOK);
      Result := False;
    end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  try
    if not Prepared then RaiseException(CustomMessage('InvalidPayload'));
    CheckTarget;
  except Result := GetExceptionMessage; end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    if not RegWriteStringValue(HKCU64, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppId}_is1',
        'DisplayVersion', ApplicationVersion) then
      RaiseException('Unable to register the installed application version.');
end;
