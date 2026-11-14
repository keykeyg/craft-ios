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


def patch_stubs(root: Path) -> None:
    cargo = root / "Cargo.toml"
    text = cargo.read_text()
    lines = []
    for crate, folder in STUBS.items():
        dest = root / "ios-shim" / folder
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(HERE / folder, dest, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
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
    app_patches(root, name)


if __name__ == "__main__":
    main()
