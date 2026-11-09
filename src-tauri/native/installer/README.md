# Native installer

`installer.nsi.hbs`, `utils.nsh`, `FileAssociation.nsh` and `English.nsh` are
the NSIS template of the Tauri bundler, copied unchanged from
`crates/tauri-bundler/src/bundle/windows/nsis` at tag `tauri-cli-v2.11.4`
(MIT / Apache-2.0, https://github.com/tauri-apps/tauri). The WebView build's
installers up to 1.2.0 were produced from that same template, so an installer
built from it upgrades an existing install in place: same uninstall key, same
install folder, same Start-menu shortcut, same running-app check.

`tools/build_native_installer.py` renders the template for the native binary
with WebView2 installation set to `skip`, adds the existing
`src-tauri/installer/clipboard-history.nsh` hook, and runs the `makensis` that
the Tauri CLI downloads to `%LOCALAPPDATA%\tauri\NSIS`.
