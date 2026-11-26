"""Package the Skol VS Code extension as a .vsix without needing npm/vsce.

Usage (from the repository root or this folder):
    python editors/vscode/build_vsix.py
Then install it:
    code --install-extension editors/vscode/skol.vsix --force
"""

import json
import os
import zipfile
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
FILES = [
    "icons/skol-light.svg",
    "icons/skol-dark.svg",
    "icons/skol.png",
    "package.json",
    "extension.js",
    "language-configuration.json",
    "syntaxes/skol.tmLanguage.json",
]

pkg = json.load(open(os.path.join(HERE, "package.json"), encoding="utf-8"))
name, publisher, version = pkg["name"], pkg["publisher"], pkg["version"]
engine = pkg["engines"]["vscode"]

manifest = f"""<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="{escape(name)}" Version="{escape(version)}" Publisher="{escape(publisher)}" />
    <DisplayName>{escape(pkg.get("displayName", name))}</DisplayName>
    <Description xml:space="preserve">{escape(pkg.get("description", ""))}</Description>
    <Tags>{escape(name)}</Tags>
    <Categories>Programming Languages</Categories>
    <GalleryFlags>Public</GalleryFlags>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="{escape(engine)}" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionDependencies" Value="" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionPack" Value="" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="workspace" />
      <Property Id="Microsoft.VisualStudio.Code.LocalizedLanguages" Value="" />
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" />
  </Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/icons/skol.png" Addressable="true" />
  </Assets>
</PackageManifest>
"""

content_types = """<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension=".json" ContentType="application/json" />
  <Default Extension=".js" ContentType="application/javascript" />
  <Default Extension=".vsixmanifest" ContentType="text/xml" />
  <Default Extension=".svg" ContentType="image/svg+xml" />
  <Default Extension=".png" ContentType="image/png" />
</Types>
"""

out = os.path.join(HERE, "skol.vsix")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", content_types)
    z.writestr("extension.vsixmanifest", manifest)
    for rel in FILES:
        z.write(os.path.join(HERE, rel), "extension/" + rel)
print(f"wrote {out} ({name} {version})")
