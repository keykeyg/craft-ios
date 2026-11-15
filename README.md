# craft-ios

Personal iOS builds of the ArtCraft suite (github.com/storytold), delivered through TestFlight.

Each app is built from the upstream source at its latest release tag, with an iOS shim:

- `shim/rfd-stub` stands in for `rfd` (no iOS backend upstream): files open through the iOS
  document picker, saves go to the app's Documents folder (Files ▸ On My iPhone), message boxes
  are UIKit alerts. Based on surfrpt1's PdfCraft iOS spike.
- `shim/apply.py` patches the checkout (rfd swap, wgpu-only eframe, `patches/<app>.json`).
- `harness/` is a small Xcode project that wraps the Rust binary so Xcode compiles the icon,
  adds the privacy manifest, signs and exports the IPA.

Build: edit `build.json` (`{"apps": ["all"]}` or e.g. `["pdf", "grid"]`) and push, or run the
`testflight` workflow by hand. Signing material is in `secrets.enc`, opened with the `CRAFT_KEY`
repository secret.

## iPhone and Duo

- `shim/ios_touch.rs` is compiled into eframe: 44 pt minimum tap targets, 8 pt spacing, capsule
  buttons and iPhone-size text, applied as floors on every app's own theme.
- winit's iOS window always fills the screen, and egui lays out inside the safe area.
- Copy and paste go through the iOS pasteboard, so they work between apps.
- No links out to the original project from inside the apps: Discord, website and GitHub
  buttons are hidden (`patches/`, and the Help-menu pass in `shim/apply.py`), such links don't
  open, and there are no update checks against its releases (updates come from TestFlight).

## Glasses mode

Plug display glasses (or any external screen) into the phone and the app moves to them, with
its desktop layout; unplug and it comes back to the phone. While it's on the glasses, the phone
is the controller (`shim/ios_glasses.rs`):

- **Trackpad**: one finger moves the cursor, tap clicks, two-finger tap right-clicks, two-finger
  drag scrolls, pinch zooms. **drag** holds the button down for dragging.
- **Touch Bar**: esc, undo, redo, cut, copy, paste, save and delete, then the app's own tools and
  commands (`shim/bars.json`, from each app's default shortcuts; `scripts/shortcuts.py` lists them).
- **Sticky modifiers**: tap ⌃ ⌥ ⇧ ⌘ once for the next click or key, again to lock (•), again to
  release.
- **keyboard** brings up the on-screen keyboard; what you type goes to the app.

## Names and icons

ArtCraft's names and logos are trademarks of the ArtCraft Team, and its brand licence asks that
modified versions drop them. These builds are modified, so they ship as the Loom Studio suite ("Loom PDF",
"Loom Photo", …) with original icons (`scripts/make_icons.py`), and the in-app names and logos are
replaced too (`shim/apply.py`, `shim/brand/`). The code remains the ArtCraft team's, under its
Apache-2.0 / MIT licences; each app bundle carries the upstream NOTICE and licence files.

| id | app | upstream |
|---|---|---|
| pdf | Loom PDF | storytold/pdfcraft |
| photo | Loom Photo | storytold/photocraft |
| vector | Loom Vector | storytold/vectorcraft |
| layout | Loom Layout | storytold/designcraft |
| raw | Loom Light | storytold/lightcraft |
| film | Loom Film | storytold/filmcraft |
| motion | Loom Motion | storytold/effectcraft |
| proto | Loom Proto | storytold/figcraft |
| write | Loom Write | storytold/wordcraft |
| grid | Loom Grid | storytold/gridcraft |
| deck | Loom Deck | storytold/deckcraft |
| audio | Loom Audio | storytold/soundcraft |
| cad | Loom CAD | storytold/cadcraft |
