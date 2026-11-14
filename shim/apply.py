#!/usr/bin/env python3
"""Turn an upstream ArtCraft checkout into an iOS build tree.

    apply.py <checkout> <upstream-name>

1. Vendors the stand-ins in shim/ (rfd: file dialogs, arboard: clipboard; neither
   has an iOS backend upstream) and patches crates.io to them.
2. Drops eframe's `glow` feature (glutin has no iOS backend; iOS draws with
   wgpu on Metal) and rewrites the few lines that only compile with it.
3. Applies patches/<upstream-name>.json, a list of {"file", "find", "replace"}
   edits; each `find` must match or the build stops, so upstream drift shows up
   as a clear error instead of a silent no-op.

The result is only meant for iOS builds: desktop dialogs in this tree no-op.
"""
import json
import re
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

# Code that names eframe's glow types, rewritten for a wgpu-only eframe.
GLOW_REWRITES = [
    (re.compile(r"fn on_exit\(&mut self,\s*_\w*:\s*Option<&eframe::glow::Context>\)"), "fn on_exit(&mut self)"),
    (re.compile(r"eframe::Renderer::Glow\b"), "eframe::Renderer::Wgpu"),
]


# crates.io crates with no iOS backend, replaced by the stand-ins in shim/.
STUBS = {"rfd": "rfd-stub", "arboard": "arboard-stub"}


def locked_version(root: Path, crate: str):
    lock = root / "Cargo.lock"
    if not lock.exists():
        return None
    found = re.findall(r'\[\[package\]\]\nname = "' + re.escape(crate) + r'"\nversion = "([^"]+)"', lock.read_text())
    if not found:
        return None
    return max(found, key=lambda v: [int(x) if x.isdigit() else 0 for x in re.split(r"[.+-]", v)])


def patch_stubs(root: Path) -> None:
    cargo = root / "Cargo.toml"
    text = cargo.read_text()
    lines = []
    for crate, folder in STUBS.items():
        dest = root / "ios-shim" / folder
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(HERE / folder, dest, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
        # Take the version the lockfile already holds, so cargo uses the patch without re-resolving.
        locked = locked_version(root, crate)
        if locked:
            manifest = dest / "Cargo.toml"
            manifest.write_text(re.sub(r'^version = "[^"]+"', f'version = "{locked}"', manifest.read_text(), count=1, flags=re.M))
        lines.append(f'{crate} = {{ path = "ios-shim/{folder}" }}')
        print(f"{crate} -> ios-shim/{folder}")
    block = "\n".join(lines)
    if "[patch.crates-io]" in text:
        text = re.sub(r"^\[patch\.crates-io\]\s*$", "[patch.crates-io]\n" + block, text, count=1, flags=re.M)
    else:
        text += "\n[patch.crates-io]\n" + block + "\n"
    cargo.write_text(text)


def manifests(root: Path):
    for p in root.rglob("Cargo.toml"):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(("vendor/", "ios-shim/", "target/")) or "-web/" in rel:
            continue
        yield p


def drop_glow(root: Path) -> None:
    feat = re.compile(r'(\s*"glow"\s*,)|(,\s*"glow"\s*)(?=\])|(\[\s*"glow"\s*\])')
    for p in manifests(root):
        lines = p.read_text().splitlines(keepends=True)
        changed = False
        for i, l in enumerate(lines):
            if re.match(r"\s*eframe\s*=", l) and '"glow"' in l:
                lines[i] = feat.sub(lambda m: "[]" if m.group(3) else "", l)
                changed = True
        if changed:
            p.write_text("".join(lines))
            print("no glow:", p.relative_to(root))
    for p in root.rglob("*.rs"):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(("vendor/", "ios-shim/", "target/")) or "-web/" in rel:
            continue
        text = p.read_text(errors="ignore")
        if "glow" not in text:
            continue
        new = text
        for rx, rep in GLOW_REWRITES:
            new = rx.sub(rep, new)
        if new != text:
            p.write_text(new)
            print("glow-free:", rel)


# winit's X11/Wayland extensions only exist on Linux and the BSDs, but the apps reach them from
# `cfg(all(unix, not(target_os = "macos")))` blocks, which iOS also matches. In an iOS-only tree
# the calls simply go.
LINUX_WINIT = [
    (re.compile(r"^[ \t]*use winit::platform::(?:x11|wayland)::[^;]*;[ \t]*\n", re.M), ""),
    (re.compile(r"\b(\w+)\.with_(?:x11|wayland)\(\);"), r"let _ = &\1;"),
]


def drop_linux_winit(root: Path) -> None:
    for p in root.rglob("*.rs"):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(("vendor/", "ios-shim/", "target/")) or "-web/" in rel:
            continue
        text = p.read_text(errors="ignore")
        if "winit::platform::" not in text:
            continue
        new = text
        for rx, rep in LINUX_WINIT:
            new = rx.sub(rep, new)
        if new != text:
            p.write_text(new)
            print("no x11/wayland:", rel)


# Wayland crates don't build for iOS (rustix lacks the socket flags they use). Some apps list them
# under `cfg(all(unix, not(target_os = "macos")))`, which iOS matches: move them to a table that
# leaves iOS out.
WAYLAND_DEPS = {"wl-clipboard-rs", "wayland-client", "wayland-protocols", "wayland-protocols-wlr",
                "wayland-backend", "wayland-cursor", "smithay-clipboard"}
# arboard's own Linux gate, copied into app code next to code that uses its Linux extensions.
ARBOARD_LINUX = 'not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))'
ARBOARD_LINUX_IOS = 'not(any(target_os = "macos", target_os = "ios", target_os = "android", target_os = "emscripten"))'


def drop_wayland(root: Path) -> None:
    for p in manifests(root):
        lines = p.read_text().splitlines(keepends=True)
        section, moved, keep = "", {}, []
        for l in lines:
            m = re.match(r"\s*\[(.*)\]\s*$", l)
            if m:
                section = m.group(1)
            name = re.match(r"\s*([A-Za-z0-9_-]+)\s*=", l)
            cfg = re.match(r"target\.'cfg\((.*)\)'\.(\w+)$", section)
            if name and name.group(1) in WAYLAND_DEPS and cfg and "unix" in cfg.group(1) and "ios" not in cfg.group(1):
                key = f"target.'cfg(all({cfg.group(1)}, not(target_os = \"ios\")))'.{cfg.group(2)}"
                moved.setdefault(key, []).append(l if l.endswith("\n") else l + "\n")
                continue
            keep.append(l)
        if moved:
            text = "".join(keep)
            for key, deps in moved.items():
                text += f"\n[{key}]\n" + "".join(deps)
            p.write_text(text)
            print("wayland deps off iOS:", p.relative_to(root), sorted(re.match(r"\s*([A-Za-z0-9_-]+)", d).group(1) for v in moved.values() for d in v))
    for p in root.rglob("*.rs"):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(("vendor/", "ios-shim/", "target/")) or "-web/" in rel:
            continue
        text = p.read_text(errors="ignore")
        if ARBOARD_LINUX in text:
            p.write_text(text.replace(ARBOARD_LINUX, ARBOARD_LINUX_IOS))
            print("arboard-linux gate off iOS:", rel)


def app_patches(root: Path, name: str) -> None:
    f = REPO / "patches" / f"{name}.json"
    if not f.exists():
        return
    for edit in json.loads(f.read_text()):
        p = root / edit["file"]
        text = p.read_text()
        if edit["find"] not in text:
            sys.exit(f"patch {f.name}: not found in {edit['file']}: {edit['find'][:80]!r}")
        p.write_text(text.replace(edit["find"], edit["replace"], edit.get("count", -1)))
        print("patched:", edit["file"])


def main() -> None:
    root, name = Path(sys.argv[1]).resolve(), sys.argv[2]
    patch_stubs(root)
    drop_glow(root)
    drop_linux_winit(root)
    drop_wayland(root)
    app_patches(root, name)


if __name__ == "__main__":
    main()
