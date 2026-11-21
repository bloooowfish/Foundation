#!/usr/bin/env python3
"""Generates THIRD_PARTY_NOTICES.txt for Foundation Windows release bundles."""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import subprocess
import sys


LICENSE_PATTERNS = (
    "LICENSE*",
    "LICENCE*",
    "COPYING*",
    "NOTICE*",
    "COPYRIGHT*",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate THIRD_PARTY_NOTICES.txt from cargo metadata."
    )
    parser.add_argument(
        "--manifest",
        default="Cargo.toml",
        help="Path to root Cargo.toml manifest.",
    )
    parser.add_argument(
        "--output",
        required=True,
        help="Output path for THIRD_PARTY_NOTICES.txt.",
    )
    parser.add_argument(
        "--target",
        default="x86_64-pc-windows-msvc",
        help="Target triple passed to cargo metadata --filter-platform.",
    )
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="Workspace package name to exclude from traversal (repeatable).",
    )
    return parser.parse_args()


def load_metadata(manifest: pathlib.Path, target: str) -> dict:
    cmd = [
        "cargo",
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--filter-platform",
        target,
        "--manifest-path",
        str(manifest),
    ]
    out = subprocess.check_output(cmd, text=True, encoding="utf-8")
    return json.loads(out)


def collect_reachable_packages(
    meta: dict, excluded_names: set[str]
) -> tuple[list[dict], list[dict]]:
    pkg_by_id = {pkg["id"]: pkg for pkg in meta["packages"]}
    node_by_id = {node["id"]: node for node in meta["resolve"]["nodes"]}
    workspace_ids = set(meta["workspace_members"])

    seeds = [
        pkg_id
        for pkg_id in workspace_ids
        if pkg_by_id[pkg_id]["name"] not in excluded_names
    ]
    visited: set[str] = set()
    queue = collections.deque(seeds)

    while queue:
        cur = queue.popleft()
        if cur in visited:
            continue
        visited.add(cur)
        node = node_by_id.get(cur)
        if not node:
            continue
        for dep in node.get("deps", []):
            dep_kinds = dep.get("dep_kinds") or []
            is_non_dev = (not dep_kinds) or any(
                dk.get("kind") in (None, "build") for dk in dep_kinds
            )
            if not is_non_dev:
                continue
            dep_id = dep["pkg"]
            dep_pkg = pkg_by_id.get(dep_id)
            if dep_pkg and dep_pkg["name"] in excluded_names:
                continue
            if dep_id not in visited:
                queue.append(dep_id)

    workspace_pkgs = sorted(
        (pkg_by_id[pid] for pid in visited if pid in workspace_ids),
        key=lambda p: p["name"].lower(),
    )
    third_party_pkgs = sorted(
        (pkg_by_id[pid] for pid in visited if pid not in workspace_ids),
        key=lambda p: (p["name"].lower(), p["version"]),
    )
    return workspace_pkgs, third_party_pkgs


def package_url(pkg: dict) -> str:
    repo = (pkg.get("repository") or "").strip()
    if repo:
        return repo
    homepage = (pkg.get("homepage") or "").strip()
    if homepage:
        return homepage
    src = (pkg.get("source") or "").strip()
    if src.startswith("git+"):
        return src[4:].split("?")[0].split("#")[0]
    if "crates.io-index" in src:
        return f"https://crates.io/crates/{pkg['name']}"
    return "-"


def find_notice_files(pkg_dir: pathlib.Path) -> list[pathlib.Path]:
    found: list[pathlib.Path] = []
    seen: set[pathlib.Path] = set()
    for pat in LICENSE_PATTERNS:
        for candidate in sorted(pkg_dir.glob(pat)):
            if candidate.is_file() and candidate not in seen:
                seen.add(candidate)
                found.append(candidate)
    licenses_subdir = pkg_dir / "LICENSES"
    if licenses_subdir.is_dir():
        for candidate in sorted(licenses_subdir.iterdir()):
            if candidate.is_file() and candidate not in seen:
                seen.add(candidate)
                found.append(candidate)
    return found


def read_text_clean(path: pathlib.Path) -> str:
    raw = path.read_text(encoding="utf-8", errors="replace")
    lines = [line.rstrip() for line in raw.replace("\r\n", "\n").replace("\r", "\n").split("\n")]
    return "\n".join(lines).strip()


def format_notices(
    workspace_pkgs: list[dict], third_party_pkgs: list[dict], target: str
) -> str:
    lines: list[str] = []
    lines.append("================================================================================")
    lines.append("Foundation — Third-Party Software Notices")
    lines.append("================================================================================")
    lines.append("")
    lines.append("Project License:")
    lines.append("  Foundation is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0).")
    lines.append("  Upstream project: Kawari (https://codeberg.org/redstrate/Kawari)")
    lines.append("  Corresponding Source: https://github.com/bloooowfish/Foundation")
    lines.append(f"  Target platform: {target}")
    lines.append("")
    lines.append("Non-Rust / Bundled Web & Script Runtimes:")
    lines.append("  - Bootstrap v5.3.8 (resources/web/static/bootstrap.min.css, bootstrap.min.js)")
    lines.append("    License: MIT License")
    lines.append("    Copyright 2011-2025 The Bootstrap Authors")
    lines.append("    Source: https://github.com/twbs/bootstrap")
    lines.append("  - Lua 5.4 (vendored into server binaries via mlua)")
    lines.append("    License: MIT License (https://www.lua.org/license.html)")
    lines.append("  - SQLite (statically compiled via libsqlite3-sys)")
    lines.append("    License: SQLite Blessing / Public Domain (https://sqlite.org/copyright.html)")
    lines.append("")
    lines.append("Non-Bundled External Software Notice:")
    lines.append("  - oodle-network-shared.dll (RAD Game Tools / Epic Games, Inc.) is proprietary")
    lines.append("    software (subject to the Unreal Engine EULA when obtained via UE tooling).")
    lines.append("    It is NOT included in this repository or release archive and is loaded dynamically")
    lines.append("    at runtime from user-supplied local files only.")
    lines.append("  - Retail FINAL FANTASY XIV game assets and SqPack data are copyright Square Enix Co., Ltd.")
    lines.append("    and are never distributed in this archive.")
    lines.append("")

    mpl_pkgs = [
        p
        for p in third_party_pkgs
        if "MPL-2.0" in (p.get("license") or "")
    ]
    if mpl_pkgs:
        lines.append("--------------------------------------------------------------------------------")
        lines.append("MPL-2.0 Source Availability Notice")
        lines.append("--------------------------------------------------------------------------------")
        lines.append("The following unmodified MPL-2.0 crates were resolved; their source code is available")
        lines.append("under the MPL-2.0 terms at their respective upstream repositories:")
        for p in mpl_pkgs:
            lines.append(f"  - {p['name']} {p['version']}: {package_url(p)}")
        lines.append("")

    lines.append("================================================================================")
    lines.append(f"Resolved Workspace Crates ({len(workspace_pkgs)})")
    lines.append("================================================================================")
    for p in workspace_pkgs:
        lic = p.get("license") or "AGPL-3.0 (see repository LICENSE)"
        lines.append(f"  - {p['name']} {p['version']} ({lic})")
    lines.append("")

    lines.append("================================================================================")
    lines.append(f"Resolved Third-Party Rust Crates ({len(third_party_pkgs)})")
    lines.append("================================================================================")
    for p in third_party_pkgs:
        lic = p.get("license") or "See crate notice/repository"
        lines.append(f"  - {p['name']} {p['version']} — {lic} — {package_url(p)}")
    lines.append("")

    # Collect and deduplicate verbatim LICENSE / COPYING / NOTICE texts by content hash.
    groups: dict[str, dict] = {}
    group_order: list[str] = []
    no_license_files: list[str] = []

    for p in third_party_pkgs:
        pkg_dir = pathlib.Path(p["manifest_path"]).parent
        notice_files = find_notice_files(pkg_dir)
        label = f"{p['name']} {p['version']}"
        if not notice_files:
            no_license_files.append(f"{label} ({p.get('license') or 'unspecified'}): {package_url(p)}")
            continue
        for nf in notice_files:
            text = read_text_clean(nf)
            if not text:
                continue
            digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
            if digest not in groups:
                groups[digest] = {
                    "file": nf.name,
                    "license": p.get("license") or "-",
                    "packages": [],
                    "text": text,
                }
                group_order.append(digest)
            if label not in groups[digest]["packages"]:
                groups[digest]["packages"].append(label)

    if no_license_files:
        lines.append("--------------------------------------------------------------------------------")
        lines.append("Crates Without On-Disk LICENSE/NOTICE File (see upstream repository)")
        lines.append("--------------------------------------------------------------------------------")
        for item in no_license_files:
            lines.append(f"  - {item}")
        lines.append("")

    lines.append("================================================================================")
    lines.append(f"Verbatim License & Notice Texts ({len(group_order)} unique texts)")
    lines.append("================================================================================")
    lines.append("")

    for idx, digest in enumerate(group_order, 1):
        grp = groups[digest]
        pkg_list = ", ".join(grp["packages"])
        lines.append(f"--- Notice #{idx} ({grp['file']} | {grp['license']}) ---")
        lines.append(f"Packages: {pkg_list}")
        lines.append("")
        lines.append(grp["text"])
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    args = parse_args()
    manifest = pathlib.Path(args.manifest).resolve()
    if not manifest.is_file():
        print(f"Manifest not found: {manifest}", file=sys.stderr)
        return 1

    meta = load_metadata(manifest, args.target)
    excluded = set(args.exclude)
    workspace_pkgs, third_party_pkgs = collect_reachable_packages(meta, excluded)
    output_text = format_notices(workspace_pkgs, third_party_pkgs, args.target)

    out_path = pathlib.Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(output_text, encoding="utf-8", newline="\n")
    print(
        f"Generated {out_path} ({len(workspace_pkgs)} workspace, "
        f"{len(third_party_pkgs)} third-party crates, {len(output_text)} bytes)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
