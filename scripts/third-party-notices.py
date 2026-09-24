#!/usr/bin/env python3
"""Write THIRD_PARTY_NOTICES.md: every crate compiled into the shipped
macOS binary, its licence, and the licence texts those licences require a
binary distribution to carry.

    python3 scripts/third-party-notices.py           # rewrite the file
    python3 scripts/third-party-notices.py --check   # fail if it is stale

Why this exists: MIT, BSD and Apache-2.0 all ask that their notice travel
with the binary, and a .app bundle is a binary distribution. The bundle
carried none until 1.0.0. make-app.sh copies the generated file into
Contents/Resources, and CI runs --check so a dependency added or bumped
without regenerating fails the build rather than shipping unacknowledged.

Only what ships: normal dependencies of the `vizz-app` binary, resolved
for both Mac targets. Build scripts, proc-macros that leave no code in the
binary are still listed (their output is in it), dev-dependencies are not.

Uses nothing but cargo metadata and the crate sources cargo has already
downloaded, so it adds no tool to the build.
"""

import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD_PARTY_NOTICES.md")
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
LICENCE_NAMES = ("license", "licence", "copying", "notice", "ofl", "ufl", "unlicense", "copyright")

# Code vizz loads or embeds that cargo does not know about.
EXTRA = """
## Not from crates.io

### Syphon.framework

Embedded in the bundle (Contents/Frameworks). Syphon is by Tom Butterworth
and Anton Marini, <https://github.com/Syphon/Syphon-Framework>, under this
licence:

```
Copyright 2010 bangnoise (Tom Butterworth) & vade (Anton Marini).
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDERS OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### NDI®

NDI® is a registered trademark of Vizrt NDI AB. vizz does not include the
NDI SDK or the NDI runtime: when NDI output or input is used, vizz loads
the NDI runtime that the user has installed from <https://ndi.video/>.
"""


def metadata(target):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def shipped(meta):
    """Package ids reachable from vizz-app through normal and build edges."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(p["id"] for p in meta["packages"] if p["name"] == "vizz-app")
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            kinds = {k["kind"] for k in dep["dep_kinds"]}
            if kinds & {None, "build"}:
                stack.append(dep["pkg"])
    return seen


def licence_files(manifest_dir):
    found = []
    for base, dirs, files in os.walk(manifest_dir):
        # Stay near the top: licence files live at the root or in a
        # fonts/ or licenses/ folder, never deep in the source tree.
        depth = os.path.relpath(base, manifest_dir).count(os.sep)
        if depth >= 1:
            dirs[:] = []
        dirs[:] = [d for d in dirs if d.lower() in ("fonts", "licenses", "licences", "license", "licence")]
        for f in sorted(files):
            low = f.lower()
            if low.startswith(LICENCE_NAMES) or (base != manifest_dir and low.endswith(".txt")):
                found.append(os.path.join(base, f))
    return sorted(found)


def main():
    check = "--check" in sys.argv
    packages = {}
    for target in TARGETS:
        meta = metadata(target)
        ids = shipped(meta)
        members = set(meta["workspace_members"])
        for p in meta["packages"]:
            if p["id"] in ids and p["id"] not in members:
                packages[(p["name"], p["version"])] = p

    rows = []
    texts = {}  # text -> list of "name version"
    missing = []
    for (name, version), p in sorted(packages.items()):
        lic = p.get("license") or ("see " + p["license_file"] if p.get("license_file") else "UNKNOWN")
        rows.append(f"| {name} | {version} | {lic} |")
        files = licence_files(os.path.dirname(p["manifest_path"]))
        if not files:
            missing.append(f"{name} {version} ({lic})")
        for f in files:
            with open(f, encoding="utf-8", errors="replace") as fh:
                text = fh.read().strip()
            texts.setdefault(text, []).append(f"{name} {version} — {os.path.basename(f)}")

    lines = [
        "# Third-party notices",
        "",
        "vizz is built on the open-source work below. This file lists every crate",
        "compiled into the macOS app, with its licence, followed by the licence and",
        "copyright texts those licences ask a binary distribution to carry. It is",
        "generated by `scripts/third-party-notices.py`; do not edit it by hand.",
        "",
        f"{len(rows)} crates.",
        "",
        "| Crate | Version | Licence |",
        "| --- | --- | --- |",
        *rows,
        "",
        EXTRA.strip(),
        "",
        "## Licence texts",
        "",
        "Each text is given once, followed by the crates that carry it.",
    ]
    if missing:
        lines += [
            "",
            "These crates ship no licence file of their own; their licence is the",
            "standard text of the SPDX licence named in the table, which appears below",
            "under the crates that do ship it:",
            "",
            *[f"- {m}" for m in missing],
        ]
    for text, users in sorted(texts.items(), key=lambda kv: kv[1][0]):
        lines += ["", "---", "", "Used by:", "", *[f"- {u}" for u in users], "", "```", text.replace("```", "'''"), "```"]
    body = "\n".join(lines) + "\n"

    if check:
        current = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if current != body:
            sys.exit("THIRD_PARTY_NOTICES.md is out of date — run python3 scripts/third-party-notices.py")
        print(f"THIRD_PARTY_NOTICES.md is current ({len(rows)} crates)")
        return
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write(body)
    print(f"wrote {OUT}: {len(rows)} crates, {len(texts)} distinct texts, {len(missing)} without a file")


if __name__ == "__main__":
    main()
