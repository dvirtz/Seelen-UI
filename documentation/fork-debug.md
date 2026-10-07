# Developing this fork without a local Rust compiler

This setup is for `dvirtz/Seelen-UI`, forked from `eythaann/Seelen-UI`. All Rust compilation happens on GitHub's
`windows-2025` x64 runner. Each Rust edit requires another push and remote build. F5 launches an existing downloaded
binary; it never recompiles. No Rust, Visual Studio Build Tools, or Windows SDK is required locally.

## Local prerequisites

- Windows x64, Git, VS Code, and the Microsoft **C/C++** extension (`ms-vscode.cpptools`). The extension supplies the
  Windows native debugger (`cppvsdbg`), which loads Rust MSVC PDBs. Rust variable rendering may be less rich than a
  dedicated Rust debugger. No compiler installation is needed.
- Microsoft Edge WebView2 Evergreen Runtime, version 110 or newer. This is a runtime, not the SDK. The build uses the
  system runtime and statically links the MSVC CRT, as specified in `.cargo/config.toml`.
- A GitHub account with Actions enabled on the fork. GitHub CLI is optional; the Actions web UI is sufficient.

Open `.github/fork-debug/debug.code-workspace` in VS Code. A workspace file keeps the launch configuration out of the
ignored `.vscode` directory. Its folder points to this checkout.

## Edit, push, build, download, debug

1. Edit your Seelen checkout and commit/push your development branch to `origin` (`dvirtz/Seelen-UI`).
2. On the fork, open **Actions → Fork debug build → Run workflow**. Select the workflow ref `master`, then enter the
   branch to compile in the **branch** input. The workflow checks out that branch and records its resolved SHA. Fork
   tooling is checked out separately from the workflow ref, so the source branch need not contain these files. With
   GitHub CLI: `gh workflow run fork-debug.yml --repo dvirtz/Seelen-UI --ref master -f branch=feature/my-change`.
3. Wait for a successful run. Download `seelen-debug-x64-<full-SHA>` from that run's artifacts. Extract its contents
   directly into `target/fork-debug/`: `seelen-ui.exe` must be at `target/fork-debug/seelen-ui.exe`. Use an empty
   destination for each download so artifacts from different commits cannot mix.
4. Read `COMMIT_SHA.txt` and `BUILD_INFO.json`. Check out that exact commit locally before debugging; keep edits in a
   commit or stash first. Keep each executable and its PDB from the same artifact together. `ARTIFACT_SHA256SUMS.txt`
   hashes the downloaded files; `GENERATED_INPUTS.diff` records generated tracked changes.
5. Quit any installed Seelen instance and its service through their normal shutdown controls. An existing instance can
   cause the downloaded executable to forward its request to the installed app and exit. The downloaded build uses
   Seelen's normal settings/data directories, so back up your settings before experimental changes.
6. Set a source breakpoint in `src/background/main.rs` on the `SeelenLogger::init()` call, then run **Seelen: launch
   downloaded debug build** with F5. Verify the breakpoint becomes solid and execution stops on that Rust line. This
   early breakpoint runs before service startup; resume promptly for startup to complete.
7. Allow the normal UAC prompt for Seelen's helper service. The app can start the adjacent `slu-service.exe` without an
   installer. The service runs elevated; attaching to it may require VS Code running as administrator. Use **Seelen:
   attach to downloaded binary or service** to select the correct process. Check its executable path points to this
   artifact rather than an installed copy. Long pauses can cause the service's existing watchdog to stop; restart the
   app/service after such a pause.
8. Focus a widget and press **Ctrl+Shift+I** for its WebView DevTools. In **Sources**, enable JavaScript source maps and
   set a TypeScript/Svelte breakpoint. The frontend is embedded with unminified bundles, linked `.map` files, and
   `sourcesContent`. The artifact also contains the same bundles/maps under `frontend/` for inspection.
9. Edit again, push again, and repeat. A local source edit does not change the downloaded Rust executable or its
   embedded frontend. The existing `npm run dev` command compiles Rust and is not part of this workflow.

## Build choices and troubleshooting

The workflow reuses `.github/actions/setup` (Node 24, Deno 2, the repository's pinned nightly Rust, npm/Cargo caches)
and the existing `npm ci`, `npm run build:ui`, and hook DLL build steps. It builds all three application binaries and
the hook DLL with the dev profile: full debug information, no optimization, no stripping, incremental compilation. A
second cache saves recent target outputs per run; the shared setup also caches dependencies and target data. Caches
accelerate builds but are not downloadable runtime artifacts.

Unlike `tauri build --debug`, direct `cargo build --bins` does not enable `custom-protocol`. This matters: Seelen's
integrity check uses `tauri::is_dev()` to accept the unsigned debug checksum marker. `TAURI_CONFIG` clears `devUrl`, so
Tauri embeds `dist` even in development mode and no local HTTP server is needed. The frontend is built without
`--production`, which preserves source maps. DevTools are already enabled in `src/Cargo.toml`. No signing keys,
installer, updater publication, translation command, or release build is used.

The artifact contains `seelen-ui.exe`, `slu.exe`, `slu-service.exe`, `sluhk.dll`, each matching `.pdb`, `static/`,
`SHA256SUMS`, the debug `SHA256SUMS.sig` marker, frontend bundles/maps, the license, the exact source SHA and build
metadata. The external WebView2 Evergreen Runtime must already be installed locally.

If a breakpoint remains hollow, inspect the Debug Console/module symbol status, verify the correct process and PDB, and
compare `git rev-parse HEAD` with `COMMIT_SHA.txt`. `requireExactSource` deliberately checks source consistency.
`sourceFileMap` translates the runner checkout paths to your workspace. If GitHub changes its workspace location,
replace the mapping key with `sourceRoot` from `BUILD_INFO.json`. Registry dependencies and Rust standard library
sources are not included; these mappings cover this repository, including `libs/`.

Relevant debugger settings are documented in the
[VS Code launch configuration reference](https://code.visualstudio.com/docs/cpp/launch-json-reference). Tauri's
[context generator](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-codegen/src/context.rs) explains embedding
assets when `devUrl` is absent.

## Keeping tooling out of upstream PRs

Keep this setup as a dedicated fork-only commit on the fork's `master`. The workflow is dispatch-only and additionally
checks `github.repository == 'dvirtz/Seelen-UI'`. Leave upstream's existing `ci.yml` unchanged: it handles PRs targeting
`master` and reusable CI, runs frontend checks on Ubuntu and Rust lint/tests on Windows, and uploads no runnable builds.
Release/nightly workflows do upload binaries, but use optimized release builds and signing-related steps.

Start contribution branches from upstream rather than fork master:

```powershell
git fetch upstream
git switch -c feature/my-change upstream/master
# Make and commit only the product changes, then push to origin.
git push -u origin feature/my-change
```

Build that branch with the workflow on fork `master`. To use local debugger tooling while on a clean contribution
branch, save this workspace file and guide outside the checkout first, or keep a separate checkout of fork master;
adjust the workspace folder to the contribution checkout. Before opening an upstream PR, review
`git diff --name-only upstream/master...HEAD`: exclude the fork workflow, `.github/fork-debug/`, and this guide. If a
feature was started on fork master, recreate it from upstream and cherry-pick only its product commits.

## Validation record

The setup must be validated with a real remote build, extraction, local startup, and a Rust source breakpoint that both
resolves and is hit. Artifact packaging assertions alone do not establish runtime or debugger success. Record the
workflow run URL, source SHA, startup result, and breakpoint result when validation is performed.
