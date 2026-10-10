#!/usr/bin/env python3
"""Draws docs/cards/*.txt as docs/img/*.svg, the example cards the docs show.

A card is a Soos document with its results written beside it, in the layout
of the README's Quick guide: `expression  result`, split at the last run of
two spaces. A blank line is a blank line. `cargo test` runs every card
through the engine, so a result written here is the one the app shows; this
script only draws.

    python3 scripts/make-cards.py           write docs/img/*.svg
    python3 scripts/make-cards.py --check   exit 1 if any is out of date (CI)

Standard library only.
"""

import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARDS = ROOT / "docs" / "cards"
IMG = ROOT / "docs" / "img"

# The app's dark palette (crates/soos-app/src/style.rs).
BACKGROUND = "#222428"
PLAIN = "#f0f0f0"
RESULT = "#96c85a"
KEYWORD = "#5bc8e8"
SECONDARY = "#b8bcc3"
COMMENT = "#959aa2"
BAR = "#4a4d52"
EDGE = "#3a3d42"

FONT = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', monospace"
SIZE = 15
# Wider than a monospace glyph really is (0.6 em), so a fallback font that
# runs a little wide still leaves the gap between a line and its result.
CHAR = 9.3
LINE = 25
PAD = 22
BAR_HEIGHT = 34
GAP = 4
MIN_WIDTH = 360

# The words the app colours (crates/soos-core/src/highlight.rs).
WORDS = re.compile(
    r"\b(?:(?P<keyword>prev|sum|total|avg|average|today|tomorrow|yesterday|now)"
    r"|(?P<joining>in|to|of|on|off|as|into|before|after|plus|with|and|minus"
    r"|subtract|without|times|multiplied\s+by|mul|divide\s+by|divided\s+by|divide))\b"
)
LABEL = re.compile(r"^[^\W\d_][\w ]*:\s+")


def split_line(line):
    """(expression, result) of a card line; the result is None if it has none."""
    line = line.strip()
    expression, gap, result = line.rpartition("  ")
    return (expression.strip(), result.strip()) if gap else (line, None)


def spans(expression):
    """(text, colour) runs of an expression, coloured as the app does."""
    comment_at = expression.find("//")
    body = expression if comment_at < 0 else expression[:comment_at]
    runs, at = [], 0
    label = LABEL.match(body)
    if label:
        runs.append((label.group(0), SECONDARY))
        at = label.end()
    for word in WORDS.finditer(body, at):
        if word.start() > at:
            runs.append((body[at : word.start()], PLAIN))
        runs.append((word.group(0), KEYWORD if word.group("keyword") else SECONDARY))
        at = word.end()
    if at < len(body):
        runs.append((body[at:], PLAIN))
    if comment_at >= 0:
        runs.append((expression[comment_at:], COMMENT))
    return runs


def render(name, source):
    rows = [split_line(line) if line.strip() else None for line in source.splitlines()]
    columns = max(
        (len(row[0]) + GAP + len(row[1] or "") for row in rows if row), default=0
    )
    width = max(MIN_WIDTH, round(PAD * 2 + columns * CHAR))
    height = BAR_HEIGHT + PAD + max(len(rows) - 1, 0) * LINE + PAD
    plain = "\n".join(
        f"{row[0]}  {row[1]}" if row and row[1] else (row[0] if row else "")
        for row in rows
    )
    out = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">Soos example: {html.escape(name)}</title>',
        f'<desc id="desc">{html.escape(plain, quote=False)}</desc>',
        f'<rect x="0.5" y="0.5" width="{width - 1}" height="{height - 1}" rx="10" '
        f'fill="{BACKGROUND}" stroke="{EDGE}"/>',
    ]
    for x in (20, 38, 56):
        out.append(f'<circle cx="{x}" cy="{BAR_HEIGHT // 2}" r="6" fill="{BAR}"/>')
    out.append(
        f'<text x="{width // 2}" y="{BAR_HEIGHT // 2 + 5}" text-anchor="middle" '
        f'font-family="{FONT}" font-size="13" fill="{COMMENT}">Soos</text>'
    )
    for index, row in enumerate(rows):
        if row is None:
            continue
        y = BAR_HEIGHT + PAD + index * LINE
        runs = "".join(
            f'<tspan fill="{colour}">{html.escape(text, quote=False)}</tspan>'
            for text, colour in spans(row[0])
        )
        out.append(
            f'<text x="{PAD}" y="{y}" xml:space="preserve" font-family="{FONT}" '
            f"font-size=\"{SIZE}\">{runs}</text>"
        )
        if row[1]:
            out.append(
                f'<text x="{width - PAD}" y="{y}" text-anchor="end" '
                f'font-family="{FONT}" font-size="{SIZE}" fill="{RESULT}">'
                f"{html.escape(row[1], quote=False)}</text>"
            )
    out.append("</svg>")
    return "\n".join(out) + "\n"


def main():
    wanted = {
        IMG / f"{card.stem}.svg": render(card.stem, card.read_text(encoding="utf-8"))
        for card in sorted(CARDS.glob("*.txt"))
    }
    stale = [
        f"{path.relative_to(ROOT)}: out of date; run python3 scripts/make-cards.py"
        for path, svg in wanted.items()
        if not path.exists() or path.read_text(encoding="utf-8") != svg
    ] + [
        f"{path.relative_to(ROOT)}: no card in docs/cards; delete it"
        for path in sorted(IMG.glob("*.svg"))
        if path not in wanted
    ]
    if "--check" in sys.argv[1:]:
        for problem in stale:
            print(problem)
        return 1 if stale else 0
    IMG.mkdir(parents=True, exist_ok=True)
    for path, svg in wanted.items():
        path.write_text(svg, encoding="utf-8", newline="\n")
    for path in IMG.glob("*.svg"):
        if path not in wanted:
            path.unlink()
    return 0


if __name__ == "__main__":
    sys.exit(main())
