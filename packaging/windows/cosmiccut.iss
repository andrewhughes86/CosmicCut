; Inno Setup script for the Windows installer; the release workflow passes
; /DVersion=x.y.z /DSource=<folder with CosmicCut.exe, cosmiccut.ico, README.md, licences>.
#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{6F3C1C9E-6A3B-4D7B-9E51-2C8F7A0C1D55}
AppName=CosmicCut
AppVersion={#Version}
AppPublisher=Andy Hughes
AppPublisherURL=https://github.com/andrewhughes86/CosmicCut
DefaultDirName={autopf}\CosmicCut
DefaultGroupName=CosmicCut
DisableProgramGroupPage=yes
PrivilegesRequiredOverridesAllowed=dialog
OutputBaseFilename=CosmicCut-{#Version}-windows-x64-setup
SetupIconFile={#Source}\cosmiccut.ico
UninstallDisplayIcon={app}\CosmicCut.exe
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\CosmicCut"; Filename: "{app}\CosmicCut.exe"; IconFilename: "{app}\cosmiccut.ico"
Name: "{autodesktop}\CosmicCut"; Filename: "{app}\CosmicCut.exe"; IconFilename: "{app}\cosmiccut.ico"; Tasks: desktopicon

[Registry]
; .ccut jobs open in CosmicCut.
Root: HKA; Subkey: "Software\Classes\.ccut"; ValueType: string; ValueName: ""; ValueData: "CosmicCut.Job"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\CosmicCut.Job"; ValueType: string; ValueName: ""; ValueData: "CosmicCut job"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\CosmicCut.Job\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\cosmiccut.ico"
Root: HKA; Subkey: "Software\Classes\CosmicCut.Job\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\CosmicCut.exe"" ""%1"""

[Run]
Filename: "{app}\CosmicCut.exe"; Description: "{cm:LaunchProgram,CosmicCut}"; Flags: nowait postinstall skipifsilent
