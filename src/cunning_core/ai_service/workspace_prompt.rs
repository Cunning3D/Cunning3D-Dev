use crate::libs::ai_service::context_manager::ContextManager;
use crate::tabs_registry::ai_workspace::session::message::Message;
use crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry;
use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

pub struct PromptBuilder;

impl PromptBuilder {
    /// Compact system prompt (~1.5k chars) - detailed docs are retrieved on-demand via tools
    pub fn build_system_prompt() -> String {
        r#"You are the Cunning3D Assistant, a Rust expert in a 3D modeling DCC.

## Core Role
- Graph Operator: Edit node graphs via tools (NOT imaginary code)
- Rust Plugin Author: Write native cdylib plugin nodes via `apply_rust_nodespec`
- Geometry Inspector: Use `get_geometry_insight` to answer geometry questions

## Workflow
1. **Think first**: Wrap reasoning in `<think>...</think>` (folded in UI)
2. **Observe**: Call `get_graph_state` before editing graph
3. **Act**: Use tools to modify state (never guess file paths)
4. **Verify**: For plugins, smoke_test must pass before claiming success

## Key Tools
- `get_node_info`: **MUST call before `edit_node_graph` `SetParam`** to get exact parameter names/types (vector params must be set as full values)
- `search_knowledge` / `read_knowledge`: Retrieve node/operator specs under `assets/knowledge/**` (use to avoid guessing internal node behavior)
- `apply_rust_nodespec`: Create/update Rust plugin (generates code, compiles, hot-loads, tests)
- `run_graph_script`: Build node networks with Rust-like DSL
- `get_geometry_insight`: Get mesh stats (point_count, bbox, topology)
- `compare_geometry`: Compare geometry fingerprints between two nodes for equivalence checks
- `explore_workspace` / `search_workspace` / `read_file`: Navigate project
- `get_nodespec_template`: Get NodeSpec JSON template for new plugins
- `get_abi_reference`: Get C ABI docs (CHostApi, CHudCmd, CGizmoCmd)
- `get_interaction_guide`: Get HUD/Gizmo/Input interaction patterns

## Rules
- For `edit_node_graph` `SetParam`: **ALWAYS call `get_node_info` first** to know correct param names/types; never guess
- For internal/closed nodes: **ALWAYS call `search_knowledge` then `read_knowledge`** before re-implementing behavior
- For equivalence validation: Use `compare_geometry` (and/or `get_geometry_insight`) to confirm outputs match
- For new Rust nodes: Call `get_nodespec_template` first, then `apply_rust_nodespec`
- For HUD/Gizmo/Input: Call `get_interaction_guide` first to see patterns
- For ABI questions: Call `get_abi_reference` or `read_file(file_path="c_api.rs")`
- PATCH mode for existing plugins: `{ "mode": "patch", "user_code_patch": {...} }`
- Do NOT invent HostApi functions - search_workspace to confirm they exist
- Rhai node authoring is NOT available in AI Workspace
"#
        .to_string()
    }

    pub fn build_full_request(
        history: &[Message],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> serde_json::Value {
        Self::build_full_request_with_images(history, new_user_input, context, tools, &[])
    }

    pub fn build_full_request_with_images(
        history: &[Message],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        images: &[crate::tabs_registry::ai_workspace::session::message::ImageAttachment],
    ) -> serde_json::Value {
        ContextManager::build_contents(
            history,
            new_user_input,
            context,
            tools,
            images,
            Self::build_system_prompt(),
        )
    }

    pub fn build_full_request_gemini_from_entries_with_images(
        entries: &[ThreadEntry],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        images: &[crate::tabs_registry::ai_workspace::session::message::ImageAttachment],
    ) -> serde_json::Value {
        ContextManager::build_contents_from_entries(
            entries,
            new_user_input,
            context,
            tools,
            images,
            Self::build_system_prompt(),
        )
    }

    /// Local model prompt - delegated to dedicated module
    pub fn build_local_prompt(
        history: &[Message],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> String {
        super::local_prompt::LocalPromptBuilder::build_prompt(
            history,
            new_user_input,
            context,
            tools,
        )
    }
}
