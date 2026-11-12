# Disk space: what to delete in a Rust and Tauri project

Written 12 November 2026. Figures are from `/opt/templates/rust` on the day of writing.

## Summary

- The project source is small. The build output is large.
- Almost all the space is in one folder, `src-tauri/target/`, which was 2.9 GB.
- That folder is a cache. Deleting it loses nothing that cannot be rebuilt from the source. The cost is rebuild time (about 130 seconds for a full release build on this machine).
- The finished program is a single file of about 4.3 MB. Only that file is distributed.

## Why the folder gets so large

1. **Rust compiles every dependency from source.** Tauri pulls in about 260 crates (GTK bindings, the web view layer, a regex engine, a CSS parser and so on). Each one is compiled into an `.rlib` file (the compiled code) and an `.rmeta` file (type information for the compiler). Together these are several hundred megabytes even for a small app.
2. **Intermediate files are much larger than the result.** The `.rlib` files contain the compiler's intermediate form. Linking and stripping them produces the 4.3 MB program. The intermediate files are kept so later builds can be incremental.
3. **Each different set of build settings creates a second full copy.** Cargo names files with a hash of the settings used. Changing the profile (for example adding LTO or `opt-level = "s"`), the flags, or the features produces a new hash, so every dependency is compiled again and stored next to the old copy. Cargo does not remove the old copy.
4. **Cargo does not clean up after itself.** There is no automatic expiry inside `target/`.

Measured on this project: all 260 crates existed in more than one variant, and about 1.9 GB of the 2.7 GB in `deps/` belonged to older variants. The worst cases were `gtk` (3 builds), `tauri_utils` (5), `rust_scaffold_lib` (7), `gio`, `tauri`, `glib` and `syn`.

Hypothesis, not confirmed: the variants came from release profile changes, different build flags between `build.sh` and earlier manual builds, and diagnostic builds made while investigating drawing speed.

## Folders and what they hold

| Folder | Size (12 Nov 2026) | Safe to delete? | Notes |
|---|---|---|---|
| `src-tauri/target/` | 2.9 GB | Yes | Pure build cache. Recreated by the next build. |
| `src-tauri/target/release/deps/` | 2.7 GB | Yes | Compiled dependencies. This is where the duplicates are. |
| `src-tauri/target/release/build/` | 186 MB | Yes | Output of dependency build scripts. |
| `src-tauri/target/release/.fingerprint/` | 23 MB | Yes | Cargo's record of what is up to date. |
| `src-tauri/target/debug/` | not present | Yes | Appears if a debug build is ever made. Usually the largest folder. |
| `src-tauri/gen/` | small | Check first | Tauri generated files. It is in `.gitignore`. Check that nothing needs it before deleting. |
| `node_modules/` | not used here | Yes | Only exists in projects that use Node. The water tracker has 68 MB. Recreated by `npm install`. |
| `/tmp/variants/`, `/tmp/css_backup_aero/` | small | Yes | Diagnostic leftovers. `/tmp` clears on reboot anyway. |

## Folders to keep

| Folder or file | Why |
|---|---|
| `src-tauri/src/`, `ui/`, `docs/` | The source. |
| `src-tauri/Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, `capabilities/`, `icons/`, `build.rs` | Build definition. `Cargo.lock` pins exact versions. |
| `src-tauri/benchmark/` | Scripts and raw results (60 KB). |
| `backup/` | Source archives (5.7 MB). Source only, no build files. |
| `config.json` | Local settings. Regenerated with defaults if missing, but it holds the saved theme and layout. |
| The built program, `src-tauri/target/release/rust-scaffold` | 4.3 MB. Copy it elsewhere first if it is needed after cleaning. |

## Ways to reduce the size

Listed from simplest to most involved. None of these has been run yet.

1. **`cargo clean` in `src-tauri/`.** Deletes the whole `target/` folder. Next build starts fresh and leaves one copy of each dependency, expected to be roughly 1 GB for a release build. Not measured yet.
2. **Delete only `target/` after a milestone.** The source archive and the 4.3 MB program are what matter. Treat `target/` as disposable.
3. **`cargo-sweep`.** Removes only files older than a given time, or not used by the current toolchain, so the live variants stay. It is not installed, so this needs a `cargo install` (a download and a compile).
4. **Keep the build settings stable.** The duplicates came from changing settings. Pick the profile, flags and features and avoid changing them between builds. This prevents the growth in the first place.
5. **A separate build folder outside the project.** Set `CARGO_TARGET_DIR` (for example `~/.cache/cargo-targets/rust-scaffold`) so the project folder stays small and copies of the template do not carry gigabytes. Several projects can also share one target folder to avoid compiling the same dependencies repeatedly. Cost: the binary is no longer under the project folder, so `run.sh` and `build.sh` would need the new path.
6. **A `fast` profile** (`inherits = "release"`, no LTO) would make development rebuilds quicker but would create another full set of dependency files unless `dev` is used for everything. This trades disk space for time.

## Other disk use outside the project

| Location | Size | Notes |
|---|---|---|
| `~/.rustup` | 1.5 GB | The Rust toolchain. Needed. |
| `~/.cargo/registry` | 210 MB | Downloaded crate sources, shared by all Rust projects. Needed for offline rebuilds. |

## The water tracker

`/home/kim/projects/water_tracker` is 83 MB at the time of writing and has no `target/` folder. Of that, 69 MB is `tauri-app/node_modules` and 15 MB is the finished program in `bin/`. It is not known from the files here what was done earlier to bring its size down. The most likely explanation, still unconfirmed, is that its `target/` folder was deleted after the program was built and copied to `bin/`. That matches option 2 above.

## Before copying this template to a new project

Do not copy `src-tauri/target/`. Copy the source only (or an archive from `backup/`). Otherwise the new project starts at 2.9 GB.
