Param([string]$TargetDir="F:\cargo-target2\Cunning3D_1.0")
$ErrorActionPreference="Stop"
$crate=Split-Path -Parent $MyInvocation.MyCommand.Path
$crate=Split-Path -Parent $crate
Set-Location $crate
cargo build --release --target wasm32-unknown-unknown
$inWasm=Join-Path $TargetDir "wasm32-unknown-unknown\release\cunning_player.wasm"
if(!(Test-Path $inWasm)){throw "wasm not found: $inWasm"}
wasm-bindgen --out-dir .\web_output --target web $inWasm --no-typescript
if(Get-Command wasm-opt -ErrorAction SilentlyContinue){wasm-opt -O3 --strip-dwarf --strip-producers -o .\web_output\cunning_player_bg.wasm .\web_output\cunning_player_bg.wasm}
