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
; Four-part number for the file-version resource (AppVersion may carry a "-preview.N" label).
#ifndef VersionNumeric
  #define VersionNumeric AppVersion + ".0"
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
; Define BundleModels to install every file the build script staged in {SourceDir}\Models.

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
; Per-user by default: the installer runs as the invoking user (asInvoker manifest), installs to
; %LOCALAPPDATA%\Programs\CutOut and writes only HKCU. The "all users" choice is optional; only that choice
; triggers the Windows elevation prompt. With UsePreviousPrivileges (the default) an upgrade uses the
; privilege level of the existing installation, so a machine-wide install is upgraded in place (with the
; standard Windows prompt) and never ends up duplicated next to a per-user copy.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
UsePreviousPrivileges=yes
OutputDir=..\dist
OutputBaseFilename=CutOut-Windows-{#ArchLabel}-Setup
SetupIconFile=..\assets\icon\CutOut.ico
UninstallDisplayIcon={app}\CutOut.exe
UninstallDisplayName=CutOut
WizardStyle=modern
Compression=lzma2/normal
SolidCompression=no
; Windows 10 version 1903 (build 18362) or later: CutOut.exe imports DirectML.dll, which is part of
; Windows from that version; earlier builds would fail to start it.
MinVersion=10.0.18362
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
VersionInfoVersion={#VersionNumeric}
VersionInfoTextVersion={#AppVersion}
VersionInfoCompany=CutOut
VersionInfoCopyright=Copyright (C) CutOut contributors
VersionInfoProductVersion={#VersionNumeric}
VersionInfoProductTextVersion={#AppVersion}
VersionInfoProductName=CutOut
VersionInfoDescription=CutOut setup
SetupLogging=yes
#ifdef SignTool
; Authenticode signing is optional and only active when the build supplies a
; SignTool definition (see docs/WINDOWS_SECURITY.md). Nothing is signed by default.
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
; Microsoft Visual C++ 2015-2022 runtime, app-local, so a PC without the redistributable can still start CutOut.
Source: "{#SourceDir}\vcruntime140.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\vcruntime140_1.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\msvcp140.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#SourceDir}\msvcp140_1.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\LICENSES\*"; DestDir: "{app}\Licenses"; Flags: ignoreversion recursesubdirs
Source: "..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
#ifdef BundleModels
; Model weights are float data that do not compress; store them as they are (fast to build and install).
Source: "{#SourceDir}\Models\*"; DestDir: "{app}\Models"; Flags: ignoreversion nocompression
#endif

[InstallDelete]
; Model files shipped by earlier installers (1.4 GB). Removed before this version's files are copied, so an
; upgrade does not keep models the new version no longer uses; files this installer ships are written again.
Type: files; Name: "{app}\Models\birefnet-general-fp32.onnx"
Type: files; Name: "{app}\Models\birefnet-general-fp16.onnx"

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
