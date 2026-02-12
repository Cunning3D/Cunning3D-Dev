#define AppId "{{8F8C9F7D-0B8C-4D37-9F9D-1A0B68C6C3D1}}"
#define AppName "Cunning3D"
#define AppExeName "Cunning3D"
#define AppPublisher "Cunning3D"
#define AppPublisherURL "https://cunning3d.example"
#define AppSupportURL "https://cunning3d.example"
#define AppUpdatesURL "https://cunning3d.example"
#define AppxPackageName "Cunning3D.Cunning3D"

; Allow external override via ISCC /dName="Value" without requiring ISPP GetStringDef.
#ifndef Version
  #define Version "0.10.0"
#endif
#ifndef OutputDir
  #define OutputDir "{#SourceDir}\..\target"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "Cunning3D-x86_64"
#endif
#ifndef ResourcesDir
  #define ResourcesDir "{#SourceDir}"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVerName={#AppName} {#Version}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppPublisherURL}
AppSupportURL={#AppSupportURL}
AppUpdatesURL={#AppUpdatesURL}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableReadyPage=yes
AllowNoIcons=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
Compression=lzma
SolidCompression=yes
ChangesAssociations=true
MinVersion=10.0.19045
SourceDir={#ResourcesDir}
AppVersion={#Version}
VersionInfoVersion={#Version}
ShowLanguageDialog=auto
WizardStyle=modern
CloseApplications=force

#if GetEnv("CI") != ""
SignTool=Defaultsign
#endif

DefaultDirName={autopf}\{#AppName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; GroupDescription: "Additional icons:"; Flags: unchecked
Name: "associate_c3d"; Description: "Associate .c3d files with Cunning3D"; Flags: checkedonce
Name: "win11_context_menu"; Description: "Enable Windows 11 context menu entry"; Flags: checkedonce

[Files]
Source: "{#ResourcesDir}\Cunning3D.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ResourcesDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs
#ifexist "{#ResourcesDir}\Ltools\*"
Source: "{#ResourcesDir}\Ltools\*"; DestDir: "{app}\Ltools"; Flags: ignoreversion recursesubdirs createallsubdirs
#endif
Source: "{#ResourcesDir}\appx\*"; DestDir: "{app}\appx"; Flags: ignoreversion recursesubdirs createallsubdirs; BeforeInstall: RemoveAppxPackage; AfterInstall: AddAppxPackage; Check: IsWindows11OrLaterAndSelected

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall; Check: WizardNotSilent

[UninstallRun]
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-AppxPackage -Name '{#AppxPackageName}' | Remove-AppxPackage -ErrorAction SilentlyContinue"""; Check: IsWindows11OrLater; Flags: shellexec waituntilterminated runhidden

[Registry]
; --- .c3d file association (HKCU, per-user) ---
Root: HKCU; Subkey: "Software\Classes\.c3d"; ValueType: string; ValueName: ""; ValueData: "{#AppName}.Project"; Flags: uninsdeletevalue; Tasks: associate_c3d
Root: HKCU; Subkey: "Software\Classes\{#AppName}.Project"; ValueType: string; ValueName: ""; ValueData: "{#AppName} Project"; Flags: uninsdeletekey; Tasks: associate_c3d
Root: HKCU; Subkey: "Software\Classes\{#AppName}.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"",0"; Tasks: associate_c3d
Root: HKCU; Subkey: "Software\Classes\{#AppName}.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}.exe"" ""%1"""; Tasks: associate_c3d

[Code]
function WizardNotSilent: Boolean;
begin
  Result := not WizardSilent;
end;

function IsWindows11OrLater(): Boolean;
var
  V: TWindowsVersion;
begin
  GetWindowsVersionEx(V);
  Result := (V.Major >= 10) and (V.Build >= 22000);
end;

function IsWindows11OrLaterAndSelected(): Boolean;
begin
  Result := IsWindows11OrLater() and WizardIsTaskSelected('win11_context_menu');
end;

procedure AddAppxPackage();
var
  ResultCode: Integer;
begin
  ShellExec('', 'powershell.exe',
    '-NoProfile -ExecutionPolicy Bypass -Command ' +
    AddQuotes('Add-AppxPackage -Path ''' + ExpandConstant('{app}\appx\cunning3d_explorer_command_injector.appx') + ''' -ExternalLocation ''' + ExpandConstant('{app}\appx') + ''''),
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

procedure RemoveAppxPackage();
var
  ResultCode: Integer;
begin
  if not IsWindows11OrLater() then exit;
  ShellExec('', 'powershell.exe',
    '-NoProfile -ExecutionPolicy Bypass -Command ' +
    AddQuotes('Get-AppxPackage -Name ''{#AppxPackageName}'' | Remove-AppxPackage -ErrorAction SilentlyContinue'),
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

