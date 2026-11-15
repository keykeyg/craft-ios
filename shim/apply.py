#!/usr/bin/env python3
"""Turn an upstream ArtCraft checkout into an iOS build tree.

    apply.py <checkout> <upstream-name> [<brand> <display name>]

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
import subprocess
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
    lines = [l for l in lines if l not in text]  # re-runnable: don't add a patch line twice
    block = "\n".join(lines)
    if not lines:
        return
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


def clone_at(url: str, rev: str, dest: Path) -> None:
    """Shallow checkout of one commit (reused when it's already there)."""
    if (dest / ".git").exists():
        head = subprocess.run(["git", "-C", str(dest), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        if head == rev:
            return
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for args in (["init", "-q"], ["remote", "add", "origin", url], ["fetch", "-q", "--depth", "1", "origin", rev],
                 ["checkout", "-q", "FETCH_HEAD"]):
        subprocess.run(["git", "-C", str(dest), *args], check=True)


def edit_once(path: Path, marker: str, old: str, new: str, regex: bool = False) -> None:
    text = path.read_text()
    if marker in text:
        return
    out = re.sub(old, new, text, count=1, flags=re.S) if regex else text.replace(old, new, 1)
    if out == text:
        sys.exit(f"{path}: expected code not found (upstream changed?)")
    path.write_text(out)


def patch_winit(root: Path) -> None:
    """iOS windows always fill the screen. winit's iOS backend otherwise sizes the UIWindow
    from the app's desktop window size (1440x920 and the like), which boxes and clips the app."""
    cargo = root / "Cargo.toml"
    text = cargo.read_text()
    m = re.search(r'^winit = \{ git = "(https://github\.com/storytold/winit\.git)", rev = "([0-9a-f]+)" \}', text, re.M)
    done = re.search(r'^winit = \{ path = "\.\./(ios-winit-[0-9a-f]{8})" \}', text, re.M)
    if m:
        url, rev = m.groups()
        dest = root.parent / f"ios-winit-{rev[:8]}"
        clone_at(url, rev, dest)
    elif done:  # already pointed at our checkout: just (re)apply the edits
        dest = root.parent / done.group(1)
    else:
        print("winit: no storytold patch line; window sizing left as is")
        return
    edit_once(dest / "src/platform_impl/ios/window.rs", "craft-ios",
              r"let frame = match window_attributes\.inner_size \{.*?None => screen_bounds,\s*\};",
              "// craft-ios: an iOS window always fills the screen; desktop window sizes don't apply.\n        let frame = screen_bounds;",
              regex=True)
    # Glasses mode moves the window to another screen: take on that screen's pixel density too,
    # or a 1080p display would be drawn at the phone's 3x.
    edit_once(dest / "src/platform_impl/ios/window.rs", "craft-ios: screen scale",
              "        let bounds = uiscreen.bounds();\n        self.window.setFrame(bounds);\n",
              "        let bounds = uiscreen.bounds();\n        self.window.setFrame(bounds);\n"
              "        // craft-ios: screen scale follows the screen the window is on.\n"
              "        let scale = uiscreen.nativeScale();\n"
              "        if self.view.contentScaleFactor() != scale {\n"
              "            #[allow(unused_unsafe)]\n"
              "            unsafe { self.view.setContentScaleFactor(scale) };\n"
              "        }\n")
    if m:
        cargo.write_text(text.replace(m.group(0), f'winit = {{ path = "../{dest.name}" }}'))
    print("winit -> full-screen iOS windows, external screens")


def patch_craft_libs(root: Path) -> None:
    """No update checks against the original project's GitHub releases: on iOS, updates come
    through TestFlight and the App Store."""
    cargo = root / "Cargo.toml"
    text = cargo.read_text()
    revs = set(re.findall(r'git = "https://github\.com/storytold/craft-libs", rev = "([0-9a-f]+)"', text))
    if not revs:
        print("craft-libs: not used")
        return
    if len(revs) != 1:
        sys.exit(f"craft-libs: expected one revision, found {sorted(revs)}")
    rev = revs.pop()
    dest = root.parent / f"ios-craft-libs-{rev[:8]}"
    clone_at("https://github.com/storytold/craft-libs", rev, dest)
    edit_once(dest / "crates/update/src/lib.rs", "craft-ios",
              "pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, String> {\n",
              "pub fn check_for_update(app: &App, http: &Http, current_version: &str) -> Result<Option<Release>, String> {\n"
              "    // craft-ios: updates come from TestFlight / the App Store, not the original project.\n"
              "    if cfg!(target_os = \"ios\") {\n"
              "        let _ = (app, http, current_version);\n"
              "        return Ok(None);\n"
              "    }\n")
    section = '[patch."https://github.com/storytold/craft-libs"]'
    line = f'craft-update = {{ path = "../{dest.name}/crates/update" }}'
    if section in text and line not in text:
        sys.exit(f"craft-libs: {cargo} already has a {section} section")
    if line not in text:
        cargo.write_text(text.rstrip("\n") + f"\n\n{section}\n{line}\n")
    print("craft-libs -> no upstream update checks")


def bar_module(upstream: str) -> str:
    """The app's own Touch Bar row for glasses mode, from shim/bars.json."""
    bars = json.loads((HERE / "bars.json").read_text())
    items = bars.get(upstream, [])
    rows = "".join(f"    ({json.dumps(label)}, {json.dumps(keys)}),\n" for label, keys in items)
    return ("//! Generated by craft-ios shim/apply.py from shim/bars.json: this app's Touch Bar row.\n"
            f"pub(crate) const APP: &[(&str, &str)] = &[\n{rows}];\n")


def patch_egui(root: Path, upstream: str = "") -> None:
    """The touch layer (shim/ios_touch.rs) runs in eframe before each frame, and egui-winit lays
    the UI out inside the iOS safe area (clear of the camera cutout and home indicator)."""
    cargo = root / "Cargo.toml"
    text = cargo.read_text()
    revs = set(re.findall(r'git = "https://github\.com/storytold/egui\.git", rev = "([0-9a-f]+)"', text))
    done = set(re.findall(r'path = "\.\./(ios-egui-[0-9a-f]{8})/crates/', text))
    if len(revs) == 1:
        rev = revs.pop()
        dest = root.parent / f"ios-egui-{rev[:8]}"
        clone_at("https://github.com/storytold/egui.git", rev, dest)
    elif not revs and len(done) == 1:  # already pointed at our checkout: just (re)apply the edits
        dest = root.parent / done.pop()
    else:
        sys.exit(f"egui: expected one storytold/egui revision, found {sorted(revs)}")
    eframe = dest / "crates/eframe/src"
    shutil.copyfile(HERE / "ios_touch.rs", eframe / "ios_touch.rs")
    shutil.copyfile(HERE / "ios_glasses.rs", eframe / "ios_glasses.rs")
    (eframe / "ios_bar.rs").write_text(bar_module(upstream))
    edit_once(eframe / "lib.rs", "mod ios_touch;", "\nmod epi;\n",
              "\nmod epi;\n\n#[cfg(target_os = \"ios\")]\nmod ios_touch;\n"
              "#[cfg(target_os = \"ios\")]\nmod ios_glasses;\n#[cfg(target_os = \"ios\")]\nmod ios_bar;\n")
    # Before each frame: phone or desktop sizes, and input from the phone controller.
    edit_once(eframe / "native/epi_integration.rs", "ios_glasses::before_frame",
              "        let raw_input = self.prepare_raw_input(app, raw_input);\n\n        let close_requested",
              "        let raw_input = self.prepare_raw_input(app, raw_input);\n\n"
              "        #[cfg(target_os = \"ios\")]\n"
              "        let raw_input = {\n"
              "            let mut raw_input = raw_input;\n"
              "            crate::ios_glasses::before_frame(&self.egui_ctx, &mut raw_input, viewport_ui_cb.is_none());\n"
              "            raw_input\n"
              "        };\n\n"
              "        let close_requested")
    # After the app's UI: cursor on the glasses, controller on the phone.
    edit_once(eframe / "native/epi_integration.rs", "ios_glasses::after_ui",
              "                    app.ui(ui, &mut self.frame);\n",
              "                    app.ui(ui, &mut self.frame);\n"
              "                    #[cfg(target_os = \"ios\")]\n"
              "                    crate::ios_glasses::after_ui(ui.ctx());\n")
    # Each root frame: is a external screen (glasses) connected?
    edit_once(eframe / "native/wgpu_integration.rs", "ios_glasses::sync_window",
              "            egui_winit::update_viewport_info(info, &integration.egui_ctx, window, false);\n",
              "            egui_winit::update_viewport_info(info, &integration.egui_ctx, window, false);\n"
              "            #[cfg(target_os = \"ios\")]\n"
              "            if viewport_id == ViewportId::ROOT {\n"
              "                crate::ios_glasses::sync_window(event_loop, window, &integration.egui_ctx);\n"
              "            }\n")
    # Copy and paste through the iOS pasteboard (shim/arboard-stub) instead of an in-app clipboard.
    clip = dest / "crates/egui-winit/src/clipboard.rs"
    clip.write_text(clip.read_text().replace('not(any(target_os = "android", target_os = "ios"))', 'not(target_os = "android")'))
    edit_once(dest / "crates/egui-winit/src/lib.rs", "craft-ios",
              "            .then(|| Rect::from_min_size(Pos2::ZERO, screen_size_in_points));\n",
              "            .then(|| Rect::from_min_size(Pos2::ZERO, screen_size_in_points));\n"
              "        // craft-ios: on iOS the window's inner size is its safe area; start the layout\n"
              "        // there too, so nothing sits under the camera cutout or the home indicator.\n"
              "        #[cfg(target_os = \"ios\")]\n"
              "        if let (Ok(inner), Ok(outer), Some(rect)) =\n"
              "            (window.inner_position(), window.outer_position(), self.egui_input.screen_rect.as_mut())\n"
              "        {\n"
              "            let ppp = pixels_per_point(&self.egui_ctx, window);\n"
              "            let origin = Pos2::new((inner.x - outer.x) as f32 / ppp, (inner.y - outer.y) as f32 / ppp);\n"
              "            *rect = Rect::from_min_size(origin, rect.size());\n"
              "        }\n")
    # No links out to the original project from inside the apps (Discord, website, GitHub).
    edit_once(dest / "crates/egui-winit/src/lib.rs", "craft-ios: upstream links",
              "fn open_url_in_browser(_url: &str) {\n",
              "fn open_url_in_browser(_url: &str) {\n"
              "    // craft-ios: upstream links stay closed; the original project is credited in About.\n"
              "    #[cfg(target_os = \"ios\")]\n"
              "    {\n"
              "        let u = _url.to_ascii_lowercase();\n"
              "        if [\"discord.gg/\", \"getartcraft.com\", \"github.com/storytold\", \"githubusercontent.com/storytold\"]\n"
              "            .iter()\n"
              "            .any(|h| u.contains(h))\n"
              "        {\n"
              "            log::info!(\"craft-ios: not opening upstream link {_url}\");\n"
              "            return;\n"
              "        }\n"
              "    }\n")
    text = re.sub(r'^([A-Za-z0-9_-]+) = \{ git = "https://github\.com/storytold/egui\.git", rev = "[0-9a-f]+" \}',
                  lambda m: f'{m.group(1)} = {{ path = "../{dest.name}/crates/{m.group(1)}" }}', text, flags=re.M)
    cargo.write_text(text)
    print("egui -> touch layer + safe area")


# Names and logos: ArtCraft's brand licence asks modified versions to drop them.
LITERAL = re.compile(r'(?<![A-Za-z0-9_#\'])"((?:[^"\\\n]|\\.)*)"')


LINK_IDS = ("help.discord", "help.website", "help.appPage", "help.github", "help.reportIssue",
            "help.moreApps", "help.community", "help.artcraft")
MENU_CMD = re.compile(r'((?:cmd|uic)!\(\s*(?:[a-z_]+\s+)?"(' + "|".join(re.escape(i) for i in LINK_IDS)
                      + r')",\s*"[^"]*",\s*)\[[^\]]*\]')


def hide_link_menus(root: Path) -> None:
    """Help menu entries that link to the original project (Discord, website, GitHub, issue
    tracker) get no menu path, so they don't show. The project is credited in About instead."""
    n = 0
    for top in ("crates", "apps"):
        for p in (root / top).rglob("*.rs"):
            text = p.read_text(errors="replace")
            out, k = MENU_CMD.subn(lambda m: m.group(1) + "[]", text)
            if k:
                p.write_text(out)
                n += k
    print(f"menus: {n} upstream link entries hidden")


def rebrand(root: Path, brand: str, display: str) -> None:
    tokens = [(re.compile(re.escape(brand) + r"(?![A-Za-z0-9_])"), display),
              (re.compile(r"ArtCraft(?![A-Za-z0-9_])"), "Loom Studio")]

    def fix(m):
        body = m.group(1)
        if not any(rx.search(body) for rx, _ in tokens):
            return m.group(0)
        # Only human-readable text (or the bare product name); keys like "PdfCraft-1.7" stay.
        # The rule depends only on the literal, so a translation key and the UI string that
        # looks it up always change the same way.
        if " " not in body and "\\n" not in body and body not in (brand, "ArtCraft"):
            return m.group(0)
        # The Discord link still goes to the original project's server: say so.
        art = "upstream" if "discord" in body.lower() else "Loom Studio"
        body = tokens[0][0].sub(display, body)
        body = tokens[1][0].sub(art, body)
        return '"' + body + '"'

    changed = 0
    for p in list(root.glob("apps/*/src/**/*.rs")) + list(root.glob("crates/*/src/**/*.rs")):
        if "-web/" in p.as_posix():
            continue
        text = p.read_text(errors="ignore")
        if brand not in text and "ArtCraft" not in text:
            continue
        new = LITERAL.sub(fix, text)
        if new != text:
            p.write_text(new)
            changed += 1
    brand_dir = root / "docs" / "brand"
    swapped = 0
    if brand_dir.is_dir():
        for f in (HERE / "brand").iterdir():
            if (brand_dir / f.name).exists():
                shutil.copyfile(f, brand_dir / f.name)
                swapped += 1
    print(f"rebrand: {brand} -> {display} in {changed} files, {swapped} logo files replaced")


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
    brand, display = (sys.argv[3], sys.argv[4]) if len(sys.argv) > 4 else (None, None)
    patch_stubs(root)
    patch_winit(root)
    patch_egui(root, name)
    patch_craft_libs(root)
    hide_link_menus(root)
    if brand:
        rebrand(root, brand, display)
    drop_glow(root)
    drop_linux_winit(root)
    drop_wayland(root)
    app_patches(root, name)


if __name__ == "__main__":
    main()
