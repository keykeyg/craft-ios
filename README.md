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

## Names and icons

ArtCraft's names and logos are trademarks of the ArtCraft Team, and its brand licence asks that
modified versions drop them. These builds are modified, so they ship as "KG PDF", "KG Photo", …
with original icons (`scripts/make_icons.py`). The code remains the ArtCraft team's, under its
Apache-2.0 / MIT licences; each app bundle carries the upstream NOTICE and licence files.

| id | app | upstream |
|---|---|---|
| pdf | KG PDF | storytold/pdfcraft |
| photo | KG Photo | storytold/photocraft |
| vector | KG Vector | storytold/vectorcraft |
| layout | KG Layout | storytold/designcraft |
| raw | KG Raw | storytold/lightcraft |
| film | KG Film | storytold/filmcraft |
| motion | KG Motion | storytold/effectcraft |
| proto | KG Proto | storytold/figcraft |
| write | KG Write | storytold/wordcraft |
| grid | KG Grid | storytold/gridcraft |
| deck | KG Deck | storytold/deckcraft |
| audio | KG Audio | storytold/soundcraft |
| cad | KG CAD | storytold/cadcraft |
