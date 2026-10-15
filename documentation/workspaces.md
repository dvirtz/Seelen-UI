# Shared workspaces across monitor setups

Under **Settings > General**, enable **Share workspaces between monitors** to choose an existing grid when connecting a
monitor. The prompt lists disconnected monitors, workspace counts, and workspace names. Choose a grid to reuse, or
**Keep separate workspaces**. Dismissing the prompt keeps the monitor's separate grid.

Reusing a grid links the monitors to the same workspace IDs. It does not swap their grids or copy workspace definitions.
Each workspace has one name, icon, wallpaper, window list, and associated tiling tree. Adding or removing a row or
column updates all monitors referencing that grid, including disconnected monitors. Renaming a workspace changes its
single definition.

For example, link your work monitor to your home monitor's grid. Both monitor IDs continue referencing the same
workspaces. Changes at work are already present when you return home, so an already linked monitor does not prompt
again. Windows from all cells of a displaced separate grid are moved to the adopted active workspace; its old
definitions remain in the registry without duplicated window membership.

When both linked monitors are connected, their shared grid has one active workspace and one set of windows spanning the
displays. Each tiling overlay positions only windows physically on its own monitor. When a monitor is disconnected,
restoring a workspace can move its windows to a connected monitor referencing that workspace.

The separate **Switch workspaces on all monitors together** option synchronizes _independent_ grids by row and column.
Missing positions are created as needed. Both options can be enabled: shared grids keep their common active workspace,
and independent grids switch to the corresponding position.

Monitor-specific settings remain associated with monitor IDs. A shared workspace has one wallpaper; its collection is
resolved using the first connected owner in hardware ID order (or the first saved owner while disconnected).

## Inspecting the saved configuration

The file is `%LOCALAPPDATA%\com.seelen.seelen-ui\workspaces3.json`. Monitor grids contain IDs; definitions are stored
once in the top-level `workspaces` registry:

```json
{
  "monitors": {
    "home-monitor": {
      "workspaces": [["coding", "email"]],
      "active_workspace": "coding"
    },
    "work-monitor": {
      "workspaces": [["coding", "email"]],
      "active_workspace": "coding"
    }
  },
  "workspaces": {
    "coding": {
      "id": "coding",
      "name": "Coding",
      "icon": null,
      "wallpaper": null,
      "windows": [42]
    },
    "email": {
      "id": "email",
      "name": "Email",
      "icon": null,
      "wallpaper": null,
      "windows": []
    }
  },
  "pinned": [],
  "switching": false,
  "connectedMonitors": ["work-monitor"],
  "monitorNames": {
    "home-monitor": "Home display",
    "work-monitor": "Work display"
  }
}
```

If the new file does not exist, the old `workspaces2.json` is migrated on load. Workspace IDs, metadata, active
selections, and windows are retained; duplicate window handles are removed. Missing definitions and empty or malformed
grids are repaired. The next save writes `workspaces3.json`, leaving the old file intact for older builds. Inspect the
file while Seelen is running, but exit Seelen before manually editing it because it saves changes automatically.

## Validation

Regression tests cover migration, shared references, window membership, changes to offline grids, shared activation,
independent synchronization, and invalid grids:

```powershell
cargo test -p seelen-core state::workspaces::tests
cargo check
cd libs/core
deno task build:rs
deno task build:npm
cd ../..
npm run type-check
```

On Windows, test home/work reconnections, connecting both linked monitors, moving windows between linked displays,
switching independent grids with different sizes, and adding/removing rows while the other monitor is disconnected.
Check the viewer, wallpapers, toolbar, and dialog in light/dark modes and a different accent color.
