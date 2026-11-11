; The Windows installer, built with Inno Setup 6 from the Release build:
;
;   flutter build windows --release
;   iscc /DAppVersion=0.1.0 app\windows\installer\zerohop.iss
;
; It installs for the current user, with no administrator rights, as
; messengers and most small tools do; its first window offers all users
; instead. Zerohop's key and settings live in each user's own folder either
; way, and uninstalling asks before removing them. Received files are never
; touched.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define BuildDir "..\..\build\windows\x64\runner\Release"

[Setup]
; Never change: Windows tells an update from a second app by this.
AppId={{23A01E91-EB69-434E-A872-7265F9B4C678}
AppName=Zerohop
AppVersion={#AppVersion}
AppVerName=Zerohop {#AppVersion}
AppPublisher=Zerohop contributors
AppPublisherURL=https://github.com/ayubjon-dev/zerohop
AppSupportURL=https://github.com/ayubjon-dev/zerohop/issues
AppCopyright=Copyright 2026 the Zerohop authors. Apache License 2.0.
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\Zerohop
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\..\build\windows\installer
OutputBaseFilename=Zerohop-{#AppVersion}-windows-x64-setup
SetupIconFile=..\runner\resources\app_icon.ico
UninstallDisplayIcon={app}\zerohop.exe
UninstallDisplayName=Zerohop
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; Zerohop is asked to quit before its files are replaced; the Restart
; Manager is the fallback for a copy that did not hear.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
en.RemoveData=Also remove this computer's Zerohop key, your list of devices and the settings?%n%nIf you keep them, this computer stays one of your devices when Zerohop is installed again. Received files are not removed either way.
ru.RemoveData=Удалить также ключ Zerohop на этом компьютере, список ваших устройств и настройки?%n%nЕсли их оставить, после повторной установки компьютер останется в ваших устройствах. Полученные файлы не удаляются в любом случае.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BuildDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Zerohop"; Filename: "{app}\zerohop.exe"
Name: "{autodesktop}\Zerohop"; Filename: "{app}\zerohop.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\zerohop.exe"; Description: "{cm:LaunchProgram,Zerohop}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallDelete]
; The app adds itself to Send to on every start.
Type: files; Name: "{usersendto}\Zerohop.lnk"

[Code]
const
  WindowClass = 'ZEROHOP_WINDOW';
  QuitWaitMs = 10000;
  QuitStepMs = 250;

function RegisterWindowMessage(Name: String): Integer;
  external 'RegisterWindowMessageW@user32.dll stdcall';

// A running copy holds its files open, and with background mode on, closing
// its window only hides it. Every copy is asked to quit and given a moment;
// one too old to understand is left to the Restart Manager.
procedure QuitRunningCopies();
var
  Message, Waited: Integer;
  Window: HWND;
begin
  Message := RegisterWindowMessage('dev.zerohop.app.quit');
  Waited := 0;
  Window := FindWindowByClassName(WindowClass);
  while (Window <> 0) and (Waited < QuitWaitMs) do
  begin
    PostMessage(Window, Message, 0, 0);
    Sleep(QuitStepMs);
    Waited := Waited + QuitStepMs;
    Window := FindWindowByClassName(WindowClass);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  QuitRunningCopies();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    QuitRunningCopies();
  if CurUninstallStep <> usPostUninstall then
    Exit;
  // Start at sign-in is set by the app itself; it would point nowhere now.
  RegDeleteValue(HKEY_CURRENT_USER,
    'Software\Microsoft\Windows\CurrentVersion\Run', 'Zerohop');
  if UninstallSilent then
    Exit;
  if MsgBox(CustomMessage('RemoveData'), mbConfirmation,
    MB_YESNO or MB_DEFBUTTON2) = IDYES then
    DelTree(ExpandConstant('{localappdata}\Zerohop'), True, True, True);
end;
