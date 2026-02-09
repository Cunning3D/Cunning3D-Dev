# Node Editor Module Structure

This document outlines the organization of the Node Editor module, adhering to high cohesion principles. Components are grouped by their UI container (Bars vs Menus).

## Directory Structure

```
src/tabs/node_editor/
├── mod.rs                  # Main entry point / Orchestrator
├── state.rs                # Data structures
├── drawing.rs              # Canvas rendering (Nodes, Connections)
├── mathematic.rs           # Geometric helpers
│
├── events/                 # Input Handling & Event Loop
│   ├── mod.rs
│   ├── common.rs           # Shared event logic (Pan/Zoom)
│   ├── desktop.rs          # PC-specific inputs (Shortcuts, Mouse)
│   ├── tablet.rs           # Tablet-specific inputs
│   └── mobile.rs           # Mobile-specific inputs
│
├── menus/                  # Popup & Floating Menus (Container)
│   ├── mod.rs              # Logic for handling menu triggers
│   ├── context.rs          # Right-click & Search menus
│   └── radial.rs           # Radial/Ring menu implementation
│
└── bars/                   # Static Toolbars (Container)
    ├── mod.rs
    ├── toolbar.rs          # Drawing logic for Top Bar & Side Bar
    └── items/              # Tools/Items hosted on the bars
        ├── mod.rs
        ├── cut_tool.rs     # Cut tool (Scissors)
        ├── sticky_note.rs  # Sticky Note
        ├── network_box.rs  # Network Box (Grouping)
        └── promote_note.rs # Promote Note (Voice/Text Intent)
```

## Module Responsibilities

### Bars (UI Container)
Hosts static UI elements that persist on screen.
- **toolbar.rs**: Renders the layout of the Top Bar and Sidebar.
- **items/**: Self-contained logic for tools that are activated from the bars.
    - *cut_tool.rs*: The interactive cutting logic.
    - *sticky_note.rs / network_box.rs / promote_note.rs*: Logic and data structures for these entities.

### Menus (UI Container)
Hosts transient UI elements that appear on demand.
- **context.rs**: The standard context menu implementation.
- **radial.rs**: The node-specific ring menu.

### Events
Handles the flow of input events.
- **common.rs**: Platform-agnostic events.
- **desktop.rs**: Keyboard/Mouse specific patterns.

### Drawing
- **drawing.rs**: Renders the graph canvas (the actual content).
