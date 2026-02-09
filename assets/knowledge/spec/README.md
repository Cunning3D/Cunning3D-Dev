## AI Knowledge Specs (Closed-Source Friendly)

This directory contains **AI-oriented specifications** that let the AI Workspace implement **equivalent Rust plugin nodes** without requiring engine source code.

### Design goals
- **Discoverable**: searchable by node name/keywords
- **Machine-friendly**: structured TOML first, minimal prose
- **Unambiguous**: explicit contracts for ports/params, units, ranges, determinism
- **Verifiable**: each spec includes a minimal validation recipe (fingerprint/assertions)

### Directory layout
- `operators/`: shared operator contracts (attributes, coordinate conventions, data layouts)
- `nodes/`: node-level specs (inputs/outputs/params/semantics/perf/validation)
- `patterns/`: implementation templates (common loops, attribute ops, caching, GPU hints)
- `validation/`: reusable equivalence checks and tolerances
- `recipes/`: graph-level recipes (how to assemble nodes for a task)

### Spec format (TOML)
Each node spec should include at least:
- `[node]`: `name`, `category`, `summary`
- `[ports]`: stable port keys + UI labels
- `[[params]]`: `name`, `type`, `default`, `units`, `range`, `meaning`, `ui`
- `[semantics]`: algorithm summary, edge cases
- `[performance]`: big-O + memory + caching suggestions
- `[validation]`: minimal checks (fingerprint fields, tolerances, golden inputs)

### Encryption / packaging
For production builds, this tree is intended to be packed into an **encrypted knowledge pack** and accessed via AI tools (search/read) so the UI can show only excerpts while the model gets full content.

