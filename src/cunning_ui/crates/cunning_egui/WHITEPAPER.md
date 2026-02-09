### Cunning UI 白皮书（v0.1）

本白皮书定义本轮 UI 重构的目标、原则、路线图（B→A→C）、DCC 必需特性，并以“可发布 GUI 库”为硬交付物，做到**一鱼两吃**：业务推进即库推进。

---

### 1. 背景与问题定义

我们当前基于 egui（立即模式）构建 DCC UI。立即模式的核心问题不是“能不能做 UI”，而是 **静止时仍有持续开销**（CPU/提交/渲染/布局/命中），在 DCC 级工程里会放大为：
- **Idle 也刷帧**（耗电、噪声、风扇、掉电池）
- **鼠标移动就刷帧**（无意义的重建与绘制）
- **重交互区域（NodeEditor 等）每帧扫描/布局/命中**（规模上来就爆）

本轮要把系统演进为：**默认保留（retained），局部即时（immediate）**，并且做到 GPUI 级别的“事件/状态驱动”，同时在 DCC 场景上更强（超大画布/复杂交互/多工具 HUD）。

---

### 2. 北极星目标（North Star）

- **G0（体验）**：UI 静止时 **0 帧提交**（无输入、无动画、无后台 UI 任务）
- **G1（交互）**：交互时只重建必要子树；NodeEditor/Viewport 等重交互区域只在真实事件帧更新
- **G2（渲染）**：SDF + GPU Text + batching 作为默认高质量路径，稳定、可控、可扩展
- **G3（工程）**：产出可发布的 GUI 库（cunning-egui / cunning-egui-wgpu / cunning-egui-dock /（可选）cunning-bevy-egui）

---

### 3. 设计原则（必须遵守）

- **P1：鼠标移动不是重建理由**  
仅 hover enter/leave、真实输入事件、显式 dirty/dep/token、动画调度允许触发。
- **P2：时间驱动必须显式调度**  
所有动画/闪烁/呼吸效果必须通过 `request_repaint_after(dt)` 驱动；动画结束立即停止调度。
- **P3：显式无效化（invalidate）优先**  
数据变更→dep/token；时间驱动→request_repaint_after；不要“靠误调用续命”。
- **P4：性能与正确性优先**  
避免死锁；避免全局锁重入；避免高频 allocation；避免 per-frame 全量扫描。
- **P5：不引入垃圾开关**  
不靠“可控开关”逃生；用机制保证正确与性能。
- **P6：最小侵入业务**  
库能兜住的，库兜；业务只做 DCC 专属（节点图/工具/HUD）。

---

### 4. 总路线图：B→A→C（核心三阶段）

#### B：时间驱动统一调度（Animation/Timer Scheduler）

**目标**：静止 0 帧；动画 active 才刷；结束自动停。  
**完成条件**：
- 所有 hover 呼吸、闪烁、过渡动画只在 active 时 `request_repaint_after`
- 无输入且无动画时，后端不再提交新帧
- 不依赖鼠标移动续命

**产出（库层）**：
- retained 子树内 `request_repaint(_after)` ⇒ 自动标记该 retained 子树 dirty（下一帧必 rebuild）

#### A：库层事件/hover/focus 路由（Dispatch for Retained）

**目标**：cache hit 时不重建，除非事件确实影响该子树。  
**完成条件**：
- retained 子树 rebuild 触发仅来自：
  - 子树内 hover 变化（enter/leave/目标变化）
  - 子树 owner 内真实输入（press/release/scroll/keys/IME）
  - dep/token/epoch/dirty
- 鼠标移动（PointerMoved/MouseMoved）不触发 rebuild
- focus/keyboard 仅在焦点属于该子树时触发

**产出（库层）**：
- “retained replay tree”（缓存回放）具备 GPUI 风格的事件驱动行为

#### C：重交互区 retained 化（NodeEditor/Viewport/HUD）

**目标**：DCC 超越点；重交互区 idle 成本趋近 0。  
**完成条件**：
- NodeEditor：分区 retained（grid/nodes/links/hud），并且命中/几何缓存事件驱动更新
- Viewport HUD：不选中时无开销；选中/工具激活时仅局部即时
- Timeline：播放才刷；停下静止

---

### 5. DCC 必需特性（与 BAC 并行推进）

- **主题系统**：字体/缩放/间距/强调色/选中态/阴影/分隔线/状态标识分类清晰
- **Pane/Tab 系统**：插件可注入 UI；默认 retained，局部即时
- **Node Graph**：
  - 视觉：多层节点样式、阴影、边框、状态标识、分割线、选中描边、节点名、端口
  - 交互：拖拽、框选、吸附、连线、插入预览、径向菜单、快捷键
  - 性能：HitCache（bucket + z-order）/局部失效（dep/token）
- **Viewport/HUD**：工具型 HUD/提示/标注，事件驱动
- **渲染**：
  - SDF primitives 扩展（rect/circle/ellipse/curve/dashed 等）
  - batching 与 stats（instances/clip_regions/drawcalls）
  - GPU text 稳定、CJK 可控、不会卡死

---

### 6. 发布目标：cunning-ui（crate 体系）

#### 6.1 crate 与 workspace 的组织建议（以 `src/cunning_ui/` 为发布根）

建议把 `src/cunning_ui/` 作为“发布工作区根”（workspace），包含多个 crate：
- **cunning-egui**：fork 的 `egui`（保留原 `lib.name = "egui"` 以保持 `use egui::…` 生态兼容）
- **cunning-egui-wgpu**：fork 的 `egui-wgpu`（SDF/GPU text/batching）
- **cunning-egui-dock**：fork 的 `egui_dock`（我们会魔改，应独立发）
- （可选）**cunning-bevy-egui**：bevy 集成（或仅内部使用，但需要边界清晰）

说明：发布名建议带 `cunning-` 前缀避免 crates.io 冲突；应用侧依赖可通过 `package = "cunning-egui"` 映射到 `use egui::`。

#### 6.2 发布 Definition of Done（发布门槛）

**行为契约**（不可破坏）：
- idle 0 帧提交（无输入/无动画）
- PointerMoved 不触发 rebuild
- retained 子树内 request_repaint(_after) ⇒ 子树 dirty ⇒ 下一帧 rebuild（动画驱动正确）
- 无死锁（锁顺序固定；禁止重入）

**工程交付**（最低限度）：
- `examples/`：最小 retained demo、SDF demo、GPU text demo、dock demo
- `CHANGELOG`：至少记录破坏性变更
- 版本策略：例如 `0.27.2-cunning.N`（跟随 upstream 基线）
- 支持范围声明：至少 Windows + wgpu 路径稳定

---

### 7. “我们什么时候算做完 UI 重构？”

当满足以下“五连”，即可宣布本轮 UI 重构完成并进入常规迭代：
- **D1**：全局静止 0 帧提交（无输入/无动画）
- **D2**：鼠标移动不刷帧（仅 hover enter/leave 或真实事件）
- **D3**：动画只在 active 期间调度，并且只 dirty 自己的 retained 子树
- **D4**：NodeEditor/Viewport/HUD/Timeline 的 idle 成本趋近 0（无全量扫描/无每帧重建）
- **D5**：cunning-ui crate 体系可发布（行为契约 + examples + 版本策略）

---

### 8. 非目标（本轮不做）

- 不追求“无痛 100% 自动迁移所有业务 UI”
- 不做“开关式绕过机制”作为长期方案
- 不做“大而全文档体系”，以可运行示例与行为契约为准


