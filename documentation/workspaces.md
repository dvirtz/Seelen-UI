# Workspaces across monitor setups

Enable **Share workspaces between monitors** under **Settings > General** to choose
a workspace grid when a monitor connects. The option is off by default.

If disconnected monitors have saved grids, a dialog lists their display names and
workspace counts and a preview of workspace names. Choose a grid to reuse, or
**Keep separate workspaces**. A new
monitor starts with a new grid; a previously used monitor starts with its saved
grid. Closing the dialog keeps that separate grid. If no disconnected grids are
available, the monitor uses its own grid without a prompt.

For example, disconnect your home monitor and connect your work monitor. Select
the home monitor's grid in the dialog. The work monitor receives the same workspace
IDs, names, rows, columns, active workspace, and window assignments. Windows already
on the work monitor's active workspace move into the reused grid's active workspace.
Tiling layouts remain associated with their workspace IDs. Saved grids and display
names persist across restarts. Restarting with the same monitors does not repeat
the prompt.

Monitors that remain connected keep their independent workspaces. Adding a second
monitor while the first remains connected gives it a separate grid, with an option
to reuse a saved grid from a disconnected monitor. Connected monitors' grids are
never offered for reuse.

Enable **Switch workspaces on all monitors together**, also under **Settings >
General**, to synchronize workspace switching. Selecting a workspace switches
each connected monitor to the same row and column. Smaller grids grow to include
the selected position. Each monitor keeps its own workspace IDs, names, windows,
and tiling layouts; disconnected monitors are left alone. This setting is off by
default and applies when you next select a workspace.

The two options are independent and can both be enabled. For example, choose to
reuse your home monitor's grid on your work monitor, and selecting its second
workspace also selects the second workspace on your connected laptop display.
With synchronization disabled, connected monitors switch independently.

If the replacement already has a saved grid, that grid is retained under the
disconnected monitor's ID. When returning home, select the work monitor's grid to
keep changes made at work. Choosing a separate grid leaves the saved grids intact.
Disabling sharing keeps current assignments and stops connection prompts.

Each newly connected monitor gets its own choice; there is no automatic pairing.
If a source reconnects or another dialog reuses its grid before you answer, the
choice is revalidated and the available grids are refreshed. Unplugging the target
monitor dismisses its pending dialog. Monitor-specific settings, including
wallpaper collection overrides, remain tied to their original monitor IDs.

## Validation

Automated regression tests live in `libs/core/src/state/workspaces/mod.rs`:

```powershell
cargo test -p seelen-core state::workspaces::tests
cargo check
cd libs/core
deno task build:rs
deno task build:npm
cd ../..
npm run type-check
```

On Windows, also verify docking with an active window and a minimized window on
different workspaces, returning to the original monitor, restarting while
undocked, and connecting two monitors. Verify synchronized switching with grids
of different sizes and with both options enabled. Verify dismissing the prompt,
unplugging while it is open, and choosing the same source from two prompts. Check
the General settings and the dialog in light and dark
mode with a different Windows accent color.
