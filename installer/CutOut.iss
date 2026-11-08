; CutOut for Windows — Inno Setup 6 script. Build with installer\build-installer.ps1
; (or: ISCC /DAppVersion=0.1.0 /DArch=x64 /DSourceDir=<staging dir> CutOut.iss).
;
; * One self-contained CutOut-Windows-<x64|ARM64>-Setup.exe; nothing else to download.
; * Installs per user by default (no administrator rights, nothing outside the
;   user profile). "Install for all users" is offered through the standard
;   Windows elevation prompt.
; * Upgrades in place (same AppId); a running CutOut is closed first.
; * Uninstall appears in Settings > Apps. User settings, the recovery file and
;   downloaded models are kept unless the user ticks the box that is offered
;   (or passes /PURGEUSERDATA when uninstalling silently).
; * No telemetry, no bundled third-party software, no network access.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
; File-name label used on the website and in SHA256SUMS.txt (x64 / ARM64).
#if Arch == "arm64"
  #define ArchLabel "ARM64"
#else
  #define ArchLabel "x64"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\win-" + Arch
#endif
; Name of the AI model file inside {SourceDir}\Models (empty = no model bundled).
#ifndef ModelFile
  #define ModelFile ""
#endif

[Setup]
; Never change the AppId: it is how upgrades and the uninstaller find an existing install.
AppId={{13785F18-068D-4D56-AE0B-6C4C624C2BFE}
AppName=CutOut
AppVersion={#AppVersion}
AppVerName=CutOut {#AppVersion}
AppPublisher=CutOut
AppPublisherURL=https://ca-works.github.io/cutout-website/
AppSupportURL=https://ca-works.github.io/cutout-website/
AppCopyright=Copyright (C) CutOut contributors
DefaultDirName={autopf}\CutOut
DefaultGroupName=CutOut
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
OutputDir=..\dist
OutputBaseFilename=CutOut-Windows-{#ArchLabel}-Setup
SetupIconFile=..\assets\icon\CutOut.ico
UninstallDisplayIcon={app}\CutOut.exe
UninstallDisplayName=CutOut
WizardStyle=modern
Compression=lzma2/normal
SolidCompression=no
; Windows 10 version 1809 or later.
MinVersion=10.0.17763
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
CloseApplications=yes
CloseApplicationsFilter=CutOut.exe
RestartApplications=no
ChangesAssociations=yes
VersionInfoVersion={#AppVersion}.0
VersionInfoProductName=CutOut
VersionInfoDescription=CutOut setup
SetupLogging=yes
#ifdef SignTool
; Authenticode signing is optional and only active when the build supplies a
; SignTool definition. Nothing is signed by default.
SignTool={#SignTool}
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked
Name: "associate"; Description: "Open CutOut &project files (.cutout) with CutOut"; GroupDescription: "File associations:"

[Files]
Source: "{#SourceDir}\CutOut.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSES\*"; DestDir: "{app}\Licenses"; Flags: ignoreversion recursesubdirs
Source: "..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion
#if ModelFile != ""
; Model weights are float data that do not compress; store them as they are (fast to build and install).
Source: "{#SourceDir}\Models\{#ModelFile}"; DestDir: "{app}\Models"; Flags: ignoreversion nocompression
#endif

[Icons]
Name: "{autoprograms}\CutOut"; Filename: "{app}\CutOut.exe"; Comment: "Remove backgrounds and edit images"
Name: "{autodesktop}\CutOut"; Filename: "{app}\CutOut.exe"; Tasks: desktopicon

[Registry]
; HKA = HKCU for per-user installs, HKLM for all-users installs.
Root: HKA; Subkey: "Software\Classes\.cutout"; ValueType: string; ValueName: ""; ValueData: "CutOut.Project"; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\CutOut.Project"; ValueType: string; ValueName: ""; ValueData: "CutOut project"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\Classes\CutOut.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\CutOut.exe,0"; Tasks: associate
Root: HKA; Subkey: "Software\Classes\CutOut.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\CutOut.exe"" ""%1"""; Tasks: associate

[Run]
Filename: "{app}\CutOut.exe"; Description: "Launch CutOut"; Flags: nowait postinstall skipifsilent

[Code]
function CmdLineParamPresent(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

// Per-user data lives outside the install folder so that upgrades and
// reinstalls never lose it. On uninstall we ask once whether to remove it.
procedure PurgeUserData();
begin
  DelTree(ExpandConstant('{userappdata}\CutOut'), True, True, True);
  DelTree(ExpandConstant('{localappdata}\CutOut'), True, True, True);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if UninstallSilent then
    begin
      if CmdLineParamPresent('/PURGEUSERDATA') then
        PurgeUserData();
    end
    else if MsgBox('Also delete your CutOut settings, the saved last session and any AI models you installed?' + #13#10 + #13#10 +
                   'Choose No to keep them for a future reinstall.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      PurgeUserData();
  end;
end;
