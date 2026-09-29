#!/usr/bin/env python3
"""Checks the repository's Markdown and source files, for CI.

- Every relative link in a Markdown file points at a file that exists, and
  every `#anchor` at a heading that exists (GitHub's heading slugs).
- No placeholder markers are left in Markdown or Rust files.

Run from anywhere; exits non-zero with one line per problem.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLACEHOLDER = re.compile(r"\b(TODO|FIXME|XXX|TBD)\b")
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
FENCE = re.compile(r"^\s*(```|~~~)")


def tracked(*patterns):
    out = subprocess.run(
        ["git", "ls-files", *patterns], cwd=ROOT, capture_output=True, text=True, check=True
    )
    return [ROOT / line for line in out.stdout.splitlines()]


def prose_lines(path):
    """(line number, text) for lines outside fenced code blocks."""
    in_fence = False
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if not in_fence:
            yield number, line


def slug(heading):
    text = re.sub(r"[`*_]", "", heading.strip().lower())
    text = re.sub(r"[^\w\- ]", "", text)
    return text.replace(" ", "-")


def anchors(path):
    found, seen = set(), {}
    for _, line in prose_lines(path):
        match = re.match(r"^#{1,6}\s+(.*)$", line)
        if not match:
            continue
        base = slug(match.group(1))
        count = seen.get(base, 0)
        seen[base] = count + 1
        found.add(base if count == 0 else f"{base}-{count}")
    return found


def main():
    problems = []
    for md in tracked("*.md"):
        for number, line in prose_lines(md):
            for target in LINK.findall(line):
                if re.match(r"^[a-z]+:", target):
                    continue
                path_part, _, anchor = target.partition("#")
                dest = (md.parent / path_part).resolve() if path_part else md
                where = f"{md.relative_to(ROOT)}:{number}"
                if not dest.exists():
                    problems.append(f"{where}: link to missing {path_part}")
                elif anchor and dest.suffix == ".md" and anchor not in anchors(dest):
                    problems.append(f"{where}: no heading for #{anchor} in {dest.name}")
    for path in tracked("*.md", "*.rs"):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if PLACEHOLDER.search(line):
                problems.append(f"{path.relative_to(ROOT)}:{number}: placeholder: {line.strip()}")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
