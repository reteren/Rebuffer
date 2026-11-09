# Releasing Rebuffer

Since 1.2.1 Rebuffer is the native build (`src-tauri/native`, Slint on top of
`src-tauri/core`); there is no WebView, npm or Tauri step anywhere.

## 1. Version

The shipping version lives in one place: `version` in
`src-tauri/native/Cargo.toml`. Every installer takes the next unused version
(`1.2.9` is followed by `1.3.0`); an installer is never rebuilt under a version
that has already produced one, and the build script refuses to overwrite one.

## 2. Checks

```
cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## 3. Installer

```
python tools/build_native_installer.py
```

Builds the release binary and writes
`src-tauri/target/release/bundle/nsis/Rebuffer_<version>_x64-setup.exe`.
It needs `makensis` and the `nsis_tauri_utils` plugin: the copies a Tauri CLI
once installed under `%LOCALAPPDATA%\tauri\NSIS` are used by default,
`NSIS_DIR` / `NSIS_PLUGIN_DIR` override them (CI does).

## 4. Try it

Quit the running copy from the tray, then run the installer (or `/S` for a
silent install; it asks for elevation, being a per-machine install), start
Rebuffer, and check: the tray icon, Alt+V, a copy landing in the popup, the
settings window, and that `HKCU\...\Run\Rebuffer` points at the installed
`rebuffer.exe` when "Launch on startup" is on.

## 5. Publish

Tag `v<version>`, create the GitHub release, attach the installer. The build
is not code-signed: SmartScreen warns on first run, so say so on the release
page.
