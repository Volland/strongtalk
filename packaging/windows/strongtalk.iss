; Inno Setup script for the Windows installer.
;   iscc /DAppVersion=2.1.0 /DSourceExe=path\to\strongtalk.exe /DOutDir=out packaging\windows\strongtalk.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\vm\target\release\strongtalk.exe"
#endif
#ifndef OutDir
  #define OutDir "..\..\out"
#endif

[Setup]
AppId={{8C1B2A44-5D0E-4D6F-9E71-2E4C1E9B7A10}
AppName=Strongtalk
AppVersion={#AppVersion}
AppVerName=Strongtalk {#AppVersion}
AppPublisher=Strongtalk Rust edition
AppPublisherURL=https://volland.github.io/strongtalk/
AppSupportURL=https://github.com/Volland/strongtalk
DefaultDirName={autopf}\Strongtalk
DefaultGroupName=Strongtalk
DisableProgramGroupPage=yes
OutputDir={#OutDir}
OutputBaseFilename=strongtalk-windows-x64-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern
UninstallDisplayIcon={app}\strongtalk.exe

[Tasks]
Name: "addtopath"; Description: "Add strongtalk to the PATH (use it from any terminal)"; GroupDescription: "Command line:"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Strongtalk Environment"; Filename: "{app}\strongtalk.exe"; Parameters: "web"; Comment: "The Strongtalk environment, in your browser"
Name: "{group}\Strongtalk to TypeScript"; Filename: "{app}\strongtalk.exe"; Parameters: "web translate"; Comment: "The TypeScript translator, in your browser"
Name: "{group}\Strongtalk Workspace (terminal)"; Filename: "{app}\strongtalk.exe"; Comment: "The terminal workspace"
Name: "{group}\Uninstall Strongtalk"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\strongtalk.exe"; Parameters: "web"; Description: "Open the Strongtalk environment now"; Flags: postinstall nowait skipifsilent

[Code]
function NeedsAddPath(Dir: string): Boolean;
var
  Current: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Current) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Current) + ';') = 0;
end;
