#!/usr/bin/env python3
"""Writes cargo-sources.json: every crate in the workspace's Cargo.lock as a Flatpak source,
so flatpak-builder can build Blobr offline.

    python3 blobr/packaging/cargo-sources.py            (from the repository root)

crates.io crates use the checksums in Cargo.lock. Git crates are fetched once as GitHub
tarballs to get their sha256 (only single-crate repos are supported, which is all the
workspace has). Run it again whenever Cargo.lock changes. Standard library only.
"""

import hashlib
import json
import sys
import tomllib
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent / "cargo-sources.json"
VENDOR = "cargo/vendor"


def crate(pkg):
    name, version, checksum = pkg["name"], pkg["version"], pkg["checksum"]
    dest = f"{VENDOR}/{name}-{version}"
    return [
        {
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        },
        {"type": "inline", "contents": json.dumps({"package": checksum, "files": {}}), "dest": dest, "dest-filename": ".cargo-checksum.json"},
    ]


def git_crate(pkg):
    """A crate from a GitHub repo pinned to a commit; returns its sources and the
    source-replacement entry Cargo needs for that repo."""
    source = pkg["source"].removeprefix("git+")
    url = urllib.parse.urlparse(source)
    commit = url.fragment
    owner, repo = url.path.strip("/").removesuffix(".git").split("/")[:2]
    if url.hostname != "github.com" or not commit:
        sys.exit(f"don't know how to fetch {pkg['name']} from {source}")
    tarball = f"https://codeload.github.com/{owner}/{repo}/tar.gz/{commit}"
    with urllib.request.urlopen(tarball) as r:
        sha256 = hashlib.sha256(r.read()).hexdigest()
    dest = f"{VENDOR}/{pkg['name']}"
    sources = [
        {"type": "archive", "archive-type": "tar-gzip", "url": tarball, "sha256": sha256, "dest": dest},
        {"type": "inline", "contents": json.dumps({"package": None, "files": {}}), "dest": dest, "dest-filename": ".cargo-checksum.json"},
    ]
    repo_url = f"https://github.com/{owner}/{repo}"
    entry = {"git": repo_url, "replace-with": "vendored-sources"}
    rev = urllib.parse.parse_qs(url.query).get("rev")
    if rev:
        entry["rev"] = rev[0]
    return sources, (repo_url, entry)


def toml_table(name, values):
    lines = [f'[source."{name}"]']
    lines += [f'{k} = "{v}"' for k, v in values.items()]
    return "\n".join(lines)


def main():
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    sources, replaced = [], {"crates-io": {"replace-with": "vendored-sources"}}
    for pkg in lock["package"]:
        src = pkg.get("source", "")
        if src.startswith("registry+"):
            sources += crate(pkg)
        elif src.startswith("git+"):
            s, (key, entry) = git_crate(pkg)
            sources += s
            replaced[key] = entry
    config = [toml_table("vendored-sources", {"directory": VENDOR})] + [toml_table(k, v) for k, v in replaced.items()]
    sources.append({"type": "inline", "contents": "\n\n".join(config) + "\n", "dest": "cargo", "dest-filename": "config"})
    OUT.write_text(json.dumps(sources, indent=1) + "\n")
    print(f"Wrote {OUT.relative_to(ROOT)} ({len(sources)} sources)")


if __name__ == "__main__":
    main()
