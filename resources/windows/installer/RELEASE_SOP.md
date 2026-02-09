## Windows release (manual download, no auto-update)

### Build installer

- **Prereqs**: Visual Studio 2022 (MSVC), Rust, Windows SDK (makeAppx), Inno Setup 6.
- **Optional signing (CI)**: Azure Trusted Signing `Invoke-TrustedSigning` available on runner.

Run from repo root:

```powershell
# Optional: embed encrypted knowledge.pack into assets/ (requires CUNNING_KNOWLEDGE_KEY)
$env:CUNNING_KNOWLEDGE_KEY="***"
.\script\bundle-windows.ps1 -Architecture x86_64 -BuildKnowledgePack

# Without knowledge.pack (dev build)
.\script\bundle-windows.ps1 -Architecture x86_64
```

Outputs:
- `target/Cunning3D-x86_64.exe`

### Publish artifacts

Compute sha256:

```powershell
Get-FileHash .\target\Cunning3D-x86_64.exe -Algorithm SHA256 | Format-List
```

Upload:
- `Cunning3D-x86_64.exe`
- sha256 value (paste on download page)

### Smoke checklist

- Install on clean Win11 VM.
- Confirm app launches from Start Menu.
- Right-click any file/folder -> **Open with Cunning3D** appears and opens Cunning3D.
- Uninstall removes the Win11 context-menu appx package.

