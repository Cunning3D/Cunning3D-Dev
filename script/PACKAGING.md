## Windows packaging (one command, F: drive)

### Goal

Build `Cunning3D-x86_64.exe` without filling up `D:` by forcing **all build outputs** to `F:\cunning3d`.

### Prereqs

- Visual Studio 2022 (MSVC)
- Rust toolchain
- Windows SDK (includes `makeAppx.exe`)
- Inno Setup 6 (`ISCC.exe`)

### Run

From workspace root:

```powershell
cd .\Cunning3D_1.0

# Default (recommended): stable release features (no CUDA/whisper)
.\script\package-windows-f.ps1 -Architecture x86_64
```

Optional:

```powershell
# Build encrypted assets/knowledge.pack (requires CUNNING_KNOWLEDGE_KEY)
$env:CUNNING_KNOWLEDGE_KEY="***"
.\script\package-windows-f.ps1 -Architecture x86_64 -BuildKnowledgePack

# Use Cargo.toml default features (may require CUDA Toolkit + CMake)
.\script\package-windows-f.ps1 -Architecture x86_64 -UseDefaultFeatures

# Change root folder on F:
.\script\package-windows-f.ps1 -Architecture x86_64 -Root "F:\c3d"
```

### Outputs

- Installer exe: `F:\cunning3d\target\Cunning3D-x86_64.exe`
- Staging dir (large): `F:\cunning3d\target\inno\x86_64\`
- Cargo target dir (large): `F:\cunning3d\cargo-target\`

### Notes

- Provider settings are stored in `%APPDATA%\Cunning3D\ai\providers.json` (not under the install folder).
- If `F:` does not exist, the script fails fast.

