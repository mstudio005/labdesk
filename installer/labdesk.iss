; LabDesk installer (Inno Setup 6). Built by the GitHub release workflow:
;   iscc /DAppVersion=1.0.0 installer\labdesk.iss
; Installing again over an older version is the update: the lab's data lives in
; %LOCALAPPDATA%\LabDesk\lab.db and is never touched by the installer or the uninstaller.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6D0C2B7A-4C1E-4B8F-9E55-1AB0D5E0C0DE}
AppName=LabDesk
AppVersion={#AppVersion}
AppPublisher=Syed Mutaib
DefaultDirName={autopf}\LabDesk
DefaultGroupName=LabDesk
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=LabDesk-Setup-{#AppVersion}
SetupIconFile=..\crates\labdesk-desktop\assets\labdesk.ico
UninstallDisplayIcon={app}\labdesk.exe
Compression=lzma2
SolidCompression=yes
; the person must read and accept the Terms of Use and Privacy before anything is installed
LicenseFile=..\TERMS.txt
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
; no administrator needed; the person can still choose "all users" if they are an admin
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Put a LabDesk icon on the desktop"; Flags: checkedonce

[Files]
Source: "..\target\release\labdesk.exe"; DestDir: "{app}"; Flags: ignoreversion
; SumatraPDF (GPLv3, https://www.sumatrapdfreader.org) prints reports straight to the default printer
Source: "..\extras\SumatraPDF.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\LabDesk"; Filename: "{app}\labdesk.exe"
Name: "{autodesktop}\LabDesk"; Filename: "{app}\labdesk.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\labdesk.exe"; Description: "Open LabDesk now"; Flags: nowait postinstall skipifsilent
; after a quiet automatic update there is nobody to press "Open LabDesk now", so open it again by itself
Filename: "{app}\labdesk.exe"; Flags: nowait; Check: ShouldRelaunch

[Code]
// Quiet updates start LabDesk again only when LabDesk asked for it (/RELAUNCH=1). When the person has just
// closed LabDesk, the update installs silently and LabDesk stays closed until they open it.
function ShouldRelaunch: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:RELAUNCH|0}') = '1');
end;
