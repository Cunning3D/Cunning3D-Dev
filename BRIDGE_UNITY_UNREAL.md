# Unity / Unreal ↔ Cunning3D Bridge 方案总结（Object Merge + 单文件 `.c3d` 容器）

本文总结当前 Unity→Cunning3D 的“类 Houdini Object Merge”方案，并给 Unreal 侧对接提供同一套协议/落地方式。

## 目标与约束（不变）

- Unity 侧 `CDA Tools` 只读查看器：对齐 Cunning3D（节点位置/连线/端口名），不做编辑/Undo/框选等。
- 白盒内省/中间节点缓存只能在 **打开 Tools 时**按需启用；关闭 Tools 时主 cook 不能被拖慢。
- “把 Unity 里选中的 CDA + 它的输入”一键带到 Cunning3D，效果类似 Houdini 的 `Object Merge`。
- 希望 **Unity 关闭后也能复现**：在 Cunning3D 内 Save 时持久化为一个可离线打开的 `.c3d` 文件（类似 `.hip`）。

## 总体结构

我们把“传数据/热更新/可复现”拆成两层：

1) **Bridge Manifest（`bridge-1`）**：一个 JSON `.c3d` 文件，描述“要打开哪个 CDA、参数、inputs 以及每个 input 对应的 blob key”。

2) **Blob Store（`.redb`）**：一个跨进程可读写的 blob 存储（基于 `redb`），用于承载实际几何/样条等大数据，并提供 `latest(key)` 语义给热更新。

Cunning3D 侧通过 **`Object Merge` 节点**读取 store 中的 `latest(blob_key)`，把“外部输入”作为图内节点（而不是占位 Input）接到 CDA 上。

## 关键行为（像 Houdini 的点）

- **打开时**：只打开“选中的 CDA + inputs”，避免把 Unity 全场景当成 session（不会因为场景里 CDA 多而全拉过来）。
- **热更新时**：Unity/Unreal 把同一个 `blob_key` 的 `latest` 指向新 blob，Cunning3D 轮询发现变更后 `mark_dirty(Object Merge)`，自动重新 cook。
- **保存时（离线复现）**：Cunning3D 的 Save/Save As 会把当前项目引用到的外部 blobs **打包进同一个 `.c3d` 文件**，从而 Unity 关闭也能打开并复现。

## 文件/数据格式

### 1) Bridge Manifest：`bridge-1`（JSON `.c3d`）

- Unity 写出一个 JSON，`header.version = "bridge-1"`。
- 内容核心是 `bridge_records[]`，每个 record 对应一个 `CunningCDAInstance`：
  - `instance_id`（稳定 ID）
  - `asset_source_path` / `asset_source_json`（用于在 Cunning3D 复原 CDA）
  - `params_json`（promoted params 的当前值）
  - `inputs[]`：每个输入包含 `kind` + `blob_key`（以及一些调试字段如 handle/dirty）

### 2) Blob Store：`.redb`

store 提供两种能力：

- `insert(blob_bytes) -> blob_id`
- `set_latest(key_hash, blob_id)` / `get_latest(key_hash) -> blob_id`

实际工程里用字符串 `blob_key`（UTF-8）做 hash（FNV-1a 64）来查 `latest`。

### 3) `.c3d` 单文件容器（用于保存/离线复现）

保存时 `.c3d` 变成“容器文件”（本质也是一个 `redb` store）：

- `c3d.project.json`：把整个 ProjectFile 的 JSON 存为 blob，并把 `latest("c3d.project.json")` 指向它。
- 被项目引用到的 `mesh_blob_key` / `spline_blob_key` 对应的 `latest(key)` blobs 也会被复制进该容器。

打开容器 `.c3d` 时，Cunning3D 会把这个容器 **挂载为当前 bridge store**，因此 `Object Merge` 读取到的是容器内打包的 blobs，实现离线复现。

## `Object Merge` 的约定（输入如何接入）

- 对每个 “mesh 输入”，Cunning3D 图里创建一个 `Object Merge` 节点：
  - 参数里保存 `mesh_blob_key`
  - cook 时读取 `latest(mesh_blob_key)` blob，反序列化为 `Geometry` 输出
- 多输入时：`Object Merge_0..n` → `Merge` → 连接到 CDA 的 inputs（与 Houdini 用法一致）

## blob key 命名（稳定且可预测）

当前约定：

- Mesh：`mesh.<instanceId>.<inputIndex>`
- Spline：`spline.<instanceId>.<inputIndex>`

要求：

- `instanceId` **必须稳定**（Unity/Unreal 组件里存一个 `ulong`，不要每次启动随机，否则热更新和保存引用会断）。
- `inputIndex` 直接对应 `inputs[]` 的索引。

## Mesh 数据：从 JSON 升级到二进制（更快更小，仍兼容）

### 新格式：`C3DG`（binary）

- Magic：`C3DG`
- Version：`1`
- Codec：`1` = `zstd(bincode(Geometry))`

`Object Merge` 会自动判断：

- blob 以 `{`/`[` 开头 → 旧的 JSON Geometry
- blob 以 `C3DG` 开头 → 新的二进制 Geometry

### Unity 写入（FFI）

优先调用：

- `cunning_geo_snapshot_bin_zstd_to_blob(handle, zstd_level)`

若 DLL 较旧没有该入口，会 fallback：

- `cunning_geo_snapshot_json_to_blob(handle)`

## Unity 侧入口（你要找的 “Open Current”）

菜单（已加）：

- `Procedural/Cunning Engine/Debug/Open Current in Cunning3D`
  - 写出稳定路径：`Library/CunningEngine/Bridge/current.c3d` + `current.redb`
  - 启动 Cunning3D：`--bridge <current.c3d> --bridge-db <current.redb> --ephemeral`
- `Procedural/Cunning Engine/Debug/Update Current Bridge (No Launch)`
  - 只更新 `current.c3d/current.redb`，不启动（用于 Cunning3D 已经开着时手动推一次更新）

备注：当前没有 IPC 去强制“已运行的 Cunning3D 实例打开某个文件”。如果检测到 Cunning3D 正在运行，会弹窗询问是否再启动一个实例。

## Unreal 侧如何复用（建议）

建议 Unreal 侧完全复用同一套“manifest + store + object merge”协议：

1) Unreal Editor 命令：`Open Current in Cunning3D`
2) 生成 `bridge-1` manifest（只包含选中 HDA/CDA + inputs）
3) 把每个输入的几何/样条 snapshot 写成 blob：
   - 最理想：复用同一套 `cunning_core_ffi`，在 Unreal 侧也持有 `geo handle`，直接调用 `cunning_geo_snapshot_bin_zstd_to_blob`
   - 如果 Unreal 侧数据结构不同：先转换/拷贝进 `cunning_core` 的 `Geometry`（或提供一个专用 FFI “从 Unreal Mesh 构建 Geometry”）
4) 同样写 `current.c3d` + `current.redb`，然后启动 Cunning3D（或未来通过 IPC 让已有实例打开）
5) 在 Cunning3D 内 Save/Save As 即可得到可离线复现的单文件 `.c3d`

## 为什么不是“纯共享内存”

共享内存确实快，但“纯共享内存”在我们的约束下不够：

- **生命周期**：Unity 关闭后共享内存往往随进程消失；而我们明确需要“可离线复现/可保存”。
- **同步/健壮性**：跨进程 ring-buffer/arena 管理、版本兼容、崩溃恢复都要自己写；还要解决“掉线/重连/重启”的状态恢复。
- **工程落地**：文件型 blob store（`redb`）在 OS page cache/mmap 下本质也接近“共享内存”，但同时天然具备持久化与可打包能力。

折中建议：

- **数据面（几何/大 blob）**：继续走 blob store（可持久化、可打包、也足够快）。
- **控制面（打开已有实例/参数热推送）**：以后可以加轻量 IPC（pipe/websocket/RPC）——不会影响 blob store 作为底座。

## 相关实现位置（方便追代码）

- Unity 菜单/写 manifest + 写 blobs：`Procedural Project/Assets/Plugins/CunningEngine/Editor/Core/CunningBridgeDebugMenu.cs`
- Unity FFI 声明：`Procedural Project/Assets/Plugins/CunningEngine/Scripts/Core/NativeMethods.cs`
- Cunning3D：`Object Merge`：`Cunning3D_1.0/src/nodes/io/object_merge.rs`
- Cunning3D：bridge store 挂载/轮询：`Cunning3D_1.0/src/bridge_db_sync.rs`
- Cunning3D：open/save（单文件容器 + 打包引用 blobs）：`Cunning3D_1.0/src/project.rs`

