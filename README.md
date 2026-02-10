# Cunning3D (Hackathon Build)

This repository contains **Cunning3D** — a Gemini Hackathon project (an experimental DCC built around a procedural modeling kernel).

## What this is

- **Gemini Hackathon project**: fast iteration and validation, while still pushing toward engineering rigor and maintainable code via agent tooling.
- **Human-written code**: the core code is authored by developers; changes are reviewed as diffs and then amplified via multi-agent collaboration.
- **Agent-assisted development**: agent tooling is used for exploration, implementation, and cross-checking, while keeping final edits reviewable, reproducible, and traceable.
- Usage ： "create a procedural house and run it everywhere, and then use gemini generate a texture for the house from 3d to uv."

## Assets (important)

- Rendering depends on WGSL shaders under `assets/shaders/`.
- If those shader assets are missing, rendering may appear blank or incorrect.

## Repo maintenance: normalize imported timestamps

If you imported older files and want to avoid timestamp ambiguity, you can run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\script\normalize-timestamps.ps1 -Baseline "2025-12-15T00:00:00" -Apply -PrintLimit 20 -LogPath .\script\normalize-timestamps.log
```

## Acknowledgements

I am grateful to the open-source community. These libraries saved an enormous amount of work:

- **Rust** — performance, safety, and tooling.
- **Bevy** — especially its **ECS** architecture and rendering foundations.
- **Manifold** — robust geometry processing primitives.
- And many other third-party libraries and their authors that made this project feasible.

## AI tooling I use

Agent-based tooling is used heavily to improve development efficiency:

- **Zed fork**: before working on Cunning3D, Zed was forked to integrate Google API key workflows; it supports concurrent agents with low memory overhead and makes diff-driven review convenient.
- **Cursor**: multi-agent development and implementation workflows.
- **Zed + Gemini + Draw.io**: rapid authoring of website and documentation.
- **Gemini Web**: Q&A and research.

## Highlights

- **Data-driven**: flexible concurrent architecture built on ECS.
- **Cross-platform**: Rust-based, designed for portability and performance.
- **License-free integration**: FFI for multiple hosts, with companion engine plugins (Unity-ready).
- **High performance**: extensive use of compute shaders and concurrent computation.
- **AI-native**: AI texture generation, self-healing node authoring, AI-driven node graph wiring, and voice-driven local models.

## References

- Referenced projects (local folder names):
  - `bevy-website-main`
  - `blender-main`
  - `gpui`
  - `Oxide-Lab`
  - `voxel-model-master`
  - `Voxy`
  - `fbxcel-develop`
  - `include`
  - `test geometry`

Special thanks:
- bevy
- zed & gpui
- bevy-ecs
- bevy-render
- egui
- egui-dock
