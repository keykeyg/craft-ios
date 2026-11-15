#!/usr/bin/env python3
"""Read an app's commands and their default keyboard shortcuts from its source.

    shortcuts.py <checkout> [--all]

Prints `id<TAB>label<TAB>shortcut` for each command that has a shortcut. Used to fill the
phone's Touch Bar in glasses mode (shim/bars.json lists which commands each app shows; this
supplies the label and key, so the bar follows the app's own keymap).
"""
import re
import sys
from pathlib import Path

KEY = r"(?:(?:Cmd|Ctrl|Shift|Alt|Opt)\+)*(?:[A-Z0-9]|F[0-9]{1,2}|Space|Enter|Return|Tab|Escape|Esc|Delete|Backspace|Plus|Minus|Equals|Comma|Period|Slash|Backslash|Semicolon|Quote|Backtick|OpenBracket|CloseBracket|Up|Down|Left|Right|Home|End|PageUp|PageDown|Insert|\[|\]|,|\.|/|;|'|=|-)"
ID = r"[a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9]+)+"

CMD = re.compile(r'cmd!\(\s*(?:[a-z_]+\s+)?"(' + ID + r')",\s*"([^"]+)",\s*\[[^\]]*\],\s*Some\("(' + KEY + r')"\)', re.S)
PAIR_KS = re.compile(r'\(\s*"(' + KEY + r')",\s*"(' + ID + r')"')
PAIR_SK = re.compile(r'\(\s*"(' + ID + r')",\s*"(' + KEY + r')"\s*[,)]')
LABEL = re.compile(r'"(' + ID + r')",\s*"([A-Z][^"]{1,40})"')


def sources(root: Path):
    for top in ("crates", "apps"):
        for p in (root / top).rglob("*.rs"):
            s = str(p)
            if "/tests" in s or s.endswith("tests.rs") or "-web/" in s or "/target/" in s:
                continue
            yield p


def extract(root: Path) -> dict:
    found: dict[str, tuple[str, str]] = {}
    labels: dict[str, str] = {}
    texts = []
    for p in sources(root):
        try:
            texts.append(p.read_text(errors="replace"))
        except OSError:
            continue
    for t in texts:
        for i, label in LABEL.findall(t):
            labels.setdefault(i, label)
    for t in texts:
        for i, label, key in CMD.findall(t):
            found.setdefault(i, (label, key))
    for t in texts:
        for key, i in PAIR_KS.findall(t):
            found.setdefault(i, (labels.get(i, ""), key))
        for i, key in PAIR_SK.findall(t):
            found.setdefault(i, (labels.get(i, ""), key))
    return found


if __name__ == "__main__":
    out = extract(Path(sys.argv[1]))
    for i in sorted(out):
        label, key = out[i]
        print(f"{i}\t{label}\t{key}")
