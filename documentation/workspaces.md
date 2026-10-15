# Shared workspaces across monitor setups

Under **Settings > Monitors > Workspaces**, enable **Share workspaces between monitors** to choose an existing grid when
connecting a monitor. The prompt lists disconnected monitors, workspace counts, and workspace names. Choose a grid to
reuse, or **Keep separate workspaces**. Dismissing the prompt keeps the monitor's separate grid.

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

## Managing monitor grids in settings

**Settings > Monitors > Workspaces** lists every saved monitor with its connection status, grid, active workspace, and
the monitors sharing that grid. Hardware IDs distinguish displays with the same name. The sharing prompt and
switch-together options live here; use the settings Save button to save those options.

Grid assignments and workspace edits apply immediately:

- Choose another monitor and select **Use this grid** to share its definitions, including between two connected monitors
  or when preparing a disconnected monitor. The automatic connection prompt still offers disconnected sources only.
- **Create separate grid** assigns a new grid containing one workspace. Windows belonging only to the displaced grid
  move into it; windows in a grid still shared by another monitor stay with that grid. Unreferenced old definitions
  remain in the registry.
- Select a workspace to rename it, activate it on a connected monitor, or remove its column (or its row when there is
  only one column). Windows from removed cells move to neighboring workspaces; the last cell cannot be removed.
- **Add column** and **Add row** grow the grid, including for disconnected monitors. Grid edits and renames affect every
  monitor sharing those workspace IDs. Editing a disconnected monitor does not activate a different workspace on
  connected monitors.

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

## Widget API compatibility

`get_virtual_desktops` and `virtual-desktops::changed` preserve the existing widget API: each monitor's `workspaces`
property contains rows of complete workspace definitions, including names and windows. This property is computed from
the canonical registry when producing a response or event; it is not stored as another set of definitions.

New consumers can use `monitor.workspaceIds` for the ID grid and resolve entries through the top-level `workspaces`
registry. Both grids have the same row/column order, and `active_workspace` remains the active workspace ID. Shared
monitors resolve their definitions from the same registry. JSON carries snapshots, so changing an object in a widget
does not mutate backend state; continue using workspace commands to make changes.

The generated TypeScript names `VirtualDesktops`, `VirtualDesktopMonitor`, and `WorkspaceGrid` describe this compatible
API shape. `WorkspaceState`, `WorkspaceMonitor`, and `WorkspaceIdGrid` describe the normalized storage model. The saved
JSON format above remains unchanged.

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
