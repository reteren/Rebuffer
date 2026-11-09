"""Builds the NSIS installer for the native (Slint) Rebuffer.

    python tools/build_native_installer.py [--skip-build]

Renders src-tauri/native/installer/installer.nsi.hbs (the Tauri bundler's own
template, see the README next to it) for the native binary and runs makensis.
The result lands where `tauri build` put its installers,
src-tauri/target/release/bundle/nsis/Rebuffer_<version>_x64-setup.exe.

The version is the shipping version, read from src-tauri/tauri.conf.json, and
the script refuses to run unless package.json and src-tauri/Cargo.toml agree:
every installer carries the next unused version, bumped in all three places.
It also refuses to overwrite an installer that already exists under that
version.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "src-tauri"
NATIVE = TAURI / "native"
TEMPLATE_DIR = NATIVE / "installer"
NSIS_HOME = Path(os.environ.get("LOCALAPPDATA", "")) / "tauri" / "NSIS"


def shipping_version() -> str:
    conf = json.loads((TAURI / "tauri.conf.json").read_text(encoding="utf-8"))["version"]
    pkg = json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"]
    cargo = re.search(r'^version = "([^"]+)"', (TAURI / "Cargo.toml").read_text(encoding="utf-8"), re.M)
    versions = {"tauri.conf.json": conf, "package.json": pkg, "Cargo.toml": cargo.group(1) if cargo else None}
    if len(set(versions.values())) != 1:
        sys.exit(f"version mismatch, bump all three places first: {versions}")
    return conf


def render(template: str, values: dict[str, str], lists: dict[str, list[str]]) -> str:
    """The subset of Handlebars the template uses with our inputs: plain
    `{{name}}`, `{{#if name}}...{{/if}}` and `{{#each name}}...{{/each}}` over
    lists of strings (`{{this}}`). Every other `#each` iterates something the
    native app does not have (resources, sidecars, file associations, deep
    links) and renders to nothing."""

    def each(m: re.Match[str]) -> str:
        name, body = m.group(1), m.group(2)
        return "".join(body.replace("{{this}}", item) for item in lists.get(name, []))

    # Innermost blocks first, so nested `each` inside an emptied block go too.
    each_re = re.compile(r"\{\{#each ([\w.]+)(?: as \|\w+\|)? ?~?\}\}((?:(?!\{\{#each).)*?)\{\{/each\}\}", re.S)
    while each_re.search(template):
        template = each_re.sub(each, template)

    def cond(m: re.Match[str]) -> str:
        return m.group(2) if values.get(m.group(1)) else ""

    template = re.sub(r"\{\{#if (\w+)\}\}(.*?)\{\{/if\}\}", cond, template, flags=re.S)

    def var(m: re.Match[str]) -> str:
        name = m.group(1)
        if name not in values:
            sys.exit(f"template variable {{{{{name}}}}} has no value")
        return values[name]

    template = re.sub(r"\{\{(\w+)\}\}", var, template)
    leftover = re.findall(r"\{\{[^}]*\}\}", template)
    if leftover:
        sys.exit(f"unrendered template syntax: {leftover[:5]}")
    return template


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-build", action="store_true", help="use the existing release binary")
    args = ap.parse_args()

    version = shipping_version()
    out_dir = TAURI / "target" / "release" / "bundle" / "nsis"
    out_file = out_dir / f"Rebuffer_{version}_x64-setup.exe"
    if out_file.exists():
        sys.exit(f"{out_file.name} already exists; an installer version is never reused")

    if not args.skip_build:
        subprocess.run(
            ["cargo", "build", "--release", "-p", "rebuffer-native"], cwd=TAURI, check=True
        )
    binary = TAURI / "target" / "release" / "rebuffer-native.exe"
    if not binary.exists():
        sys.exit(f"{binary} not found; build it or drop --skip-build")

    # Installed as rebuffer.exe: the name the WebView build installed, the Run
    # key points at and the uninstaller removes. The template copies the file
    # under its own name, so it is staged under that name first.
    stage = TAURI / "target" / "release" / "native-installer"
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    staged = stage / "rebuffer.exe"
    shutil.copy2(binary, staged)
    for name in ("utils.nsh", "FileAssociation.nsh", "English.nsh"):
        shutil.copy2(TEMPLATE_DIR / name, stage / name)

    makensis = NSIS_HOME / "makensis.exe"
    plugins = NSIS_HOME / "Plugins" / "x86-unicode" / "additional"
    if not makensis.exists() or not (plugins / "nsis_tauri_utils.dll").exists():
        sys.exit(f"NSIS from the Tauri CLI not found under {NSIS_HOME}; run `npx tauri build` once")

    values = {
        "compression": "lzma",
        "signed_plugins_path": "",
        "installer_hooks": str(TAURI / "installer" / "clipboard-history.nsh"),
        "manufacturer": "reteren",
        "product_name": "Rebuffer",
        "version": version,
        "version_with_build": f"{version}.0",
        "homepage": "",
        "install_mode": "perMachine",
        "license": "",
        "installer_icon": str(TAURI / "icons" / "icon.ico"),
        "sidebar_image": "",
        "header_image": "",
        "uninstaller_icon": "",
        "uninstaller_header_image": "",
        "main_binary_name": "rebuffer",
        "main_binary_path": str(staged),
        "bundle_id": "com.reteren.rebuffer",
        "copyright": "MIT",
        "out_file": str(out_file),
        "arch": "x64",
        "additional_plugins_path": str(plugins),
        "allow_downgrades": "true",
        "display_language_selector": "false",
        # No WebView left to install: this is the point of the native build.
        "install_webview2_mode": "skip",
        "webview2_installer_args": "",
        "webview2_bootstrapper_path": "",
        "webview2_installer_path": "",
        "minimum_webview2_version": "",
        "uninstaller_sign_cmd": "",
        "estimated_size": str(staged.stat().st_size // 1024),
        "start_menu_folder": "",
    }
    lists = {
        "languages": ["English"],
        "language_files": [str(stage / "English.nsh")],
    }
    script = render((TEMPLATE_DIR / "installer.nsi.hbs").read_text(encoding="utf-8"), values, lists)
    nsi = stage / "installer.nsi"
    # makensis reads UTF-8 with a BOM as Unicode.
    nsi.write_text(script, encoding="utf-8-sig")

    out_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(makensis), "/V3", "/INPUTCHARSET", "UTF8", str(nsi)], cwd=stage, check=True)
    print(f"built {out_file} ({out_file.stat().st_size / 1024 / 1024:.2f} MB)")


if __name__ == "__main__":
    main()
