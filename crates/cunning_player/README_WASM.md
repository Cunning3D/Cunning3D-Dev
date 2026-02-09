# cunning_player wasm build

## Build + generate `web_output`
Run from `Cunning3D_1.0/crates/cunning_player`:

```powershell
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --out-dir .\web_output --target web F:\cargo-target2\Cunning3D_1.0\wasm32-unknown-unknown\release\cunning_player.wasm --no-typescript
```

## Optional: wasm-opt
If you have `wasm-opt` in PATH:

```powershell
wasm-opt -O3 --strip-dwarf --strip-producers -o .\web_output\cunning_player_bg.wasm .\web_output\cunning_player_bg.wasm
```

## One-shot script
Use:

```powershell
.\scripts\build_wasm.ps1
```

