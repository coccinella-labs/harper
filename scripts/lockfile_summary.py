#!/usr/bin/env python3
"""Summarize Cargo.lock and MODULE.bazel.lock changes as structured facts.

This replaces the diff-scraping summary in .github/workflows/update-lockfiles.yml.
Package changes are computed by parsing the lockfiles and comparing
name -> version sets, so crates that appear at multiple versions in a single
lockfile are handled correctly.

The output is factual only. It reports what changed and, where available, the
MSRV that a candidate version declares. It deliberately does not characterize a
change as expected, safe, or a regression; that judgement belongs to a human.

Used by the Lockfiles workflow to refresh a bot-owned sticky comment on the
lockfile pull request. The pull request body is never written by this script.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11
    tomllib = None  # type: ignore[assignment]

# A crate version as it appears in a lockfile, e.g. "1.2.3" or
# "1.1.2+spec-1.1.0". Build metadata after "+" does not affect ordering.
_VERSION_RE = re.compile(r"^(\d+)(?:\.(\d+))?(?:\.(\d+))?")


@dataclass(frozen=True)
class Version:
    """A minimally ordered crate version.

    Ordering follows semver precedence for the numeric components. Build
    metadata is ignored. Pre-release identifiers are not modelled: the lockfile
    versions this script compares are all release versions, and a wrong answer
    there would be worse than an unparsed one.
    """

    raw: str

    @property
    def sort_key(self) -> tuple[int, int, int]:
        m = _VERSION_RE.match(self.raw)
        if not m:
            return (0, 0, 0)
        return (int(m.group(1)), int(m.group(2) or 0), int(m.group(3) or 0))

    def __lt__(self, other: "Version") -> bool:
        return self.sort_key < other.sort_key

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Version) and self.sort_key == other.sort_key

    def __hash__(self) -> int:
        return hash(self.sort_key)


@dataclass
class PackageChange:
    """A single crate's transition between two lockfiles."""

    name: str
    before: set[str] = field(default_factory=set)
    after: set[str] = field(default_factory=set)

    @property
    def added(self) -> set[str]:
        return self.after - self.before

    @property
    def removed(self) -> set[str]:
        return self.before - self.after

    @property
    def is_new_crate(self) -> bool:
        return not self.before and bool(self.after)

    @property
    def is_gone(self) -> bool:
        return bool(self.before) and not self.after

    @property
    def downgrades(self) -> list[tuple[str, str]]:
        """(from, to) pairs where exactly one version line moved backwards.

        A pairing is only reported when exactly one version was added and
        exactly one removed. That makes it unambiguous for a crate like `syn`
        present at two versions where one line is replaced, while a crate like
        `rand` moving two of its three versions at once falls through to
        "other version changes" -- any single pairing there would be a guess.
        """
        if len(self.added) != 1 or len(self.removed) != 1:
            return []
        (new_v,), (old_v,) = (self.added, self.removed)
        return [(old_v, new_v)] if Version(new_v) < Version(old_v) else []

    @property
    def upgrades(self) -> list[tuple[str, str]]:
        if len(self.added) != 1 or len(self.removed) != 1:
            return []
        (new_v,), (old_v,) = (self.added, self.removed)
        return [(old_v, new_v)] if Version(new_v) > Version(old_v) else []


def parse_cargo_lock(text: str) -> dict[str, set[str]]:
    """Return {crate_name: {versions}} for registry and git packages.

    Path dependencies (the workspace's own members) carry no `source` key and
    are skipped: they are versioned by the release PRs, not by lockfile
    resolution, so including them would report workspace releases as
    dependency churn.
    """
    if tomllib is None:
        raise RuntimeError("tomllib requires Python 3.11+")

    data = tomllib.loads(text)
    packages: dict[str, set[str]] = {}
    for pkg in data.get("package", []):
        name = pkg.get("name")
        version = pkg.get("version")
        if not name or not version:
            continue
        if "source" not in pkg:
            continue  # workspace member / path dependency
        packages.setdefault(name, set()).add(version)
    return packages


def diff_cargo_locks(before: dict[str, set[str]], after: dict[str, set[str]]) -> list[PackageChange]:
    changes = []
    for name in sorted(set(before) | set(after)):
        b, a = before.get(name, set()), after.get(name, set())
        if b != a:
            changes.append(PackageChange(name=name, before=b, after=a))
    return changes


def categorize(changes: list[PackageChange]) -> dict[str, list[PackageChange]]:
    """Split changes into mutually exclusive factual buckets."""
    buckets: dict[str, list[PackageChange]] = {
        "added": [],
        "removed": [],
        "upgraded": [],
        "downgraded": [],
        "version_lines": [],
    }
    for change in changes:
        if change.is_new_crate:
            buckets["added"].append(change)
        elif change.is_gone:
            buckets["removed"].append(change)
        elif change.downgrades:
            buckets["downgraded"].append(change)
        elif change.upgrades:
            buckets["upgraded"].append(change)
        else:
            # Gained or lost a version line while keeping others, e.g. a crate
            # that now appears at both 0.4.21 and 0.5.15.
            buckets["version_lines"].append(change)
    return buckets


def bazel_mod_deps(module_text: str) -> dict[str, str]:
    """Return {module_name: version} for bazel_dep declarations.

    MODULE.bazel is Starlark, not TOML, so `bazel_dep(...)` calls are matched
    rather than parsed. The pattern is anchored to the call form to avoid
    matching unrelated identifiers in comments or strings.
    """
    deps: dict[str, str] = {}
    pattern = re.compile(
        r"""^\s*bazel_dep\s*\(\s*name\s*=\s*["']([^"']+)["']\s*,\s*version\s*=\s*["']([^"']+)["']""",
        re.MULTILINE,
    )
    for match in pattern.finditer(module_text):
        deps[match.group(1)] = match.group(2)
    return deps


def bazel_lock_versions(lock_text: str) -> dict[str, str]:
    """Return {module_name: version} recorded in MODULE.bazel.lock.

    MODULE.bazel.lock is JSON. Resolved module versions are not stored as
    structured records for a module's own dependency: they appear in
    `registryFileHashes`, whose keys are the fetched module file URLs, e.g.
    "https://bcr.bazel.build/modules/rules_rust/0.74.0/MODULE.bazel". The
    version is therefore recovered from those keys.

    `moduleExtensions` entries are also walked, since extension-specific
    dependencies record explicit {name, version} objects.
    """
    try:
        data = json.loads(lock_text)
    except json.JSONDecodeError:
        return {}

    found: dict[str, str] = {}

    url_pattern = re.compile(
        r"^https?://[^/]+/modules/(?P<name>[^/]+)/(?P<version>[^/]+)/MODULE\.bazel$"
    )
    hashes = data.get("registryFileHashes")
    if isinstance(hashes, dict):
        for key in hashes:
            m = url_pattern.match(key)
            if m:
                found.setdefault(m.group("name"), m.group("version"))

    def walk(node: object) -> None:
        if isinstance(node, dict):
            name, version = node.get("name"), node.get("version")
            if isinstance(name, str) and isinstance(version, str):
                found.setdefault(name, version)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    walk(data.get("moduleExtensions", {}))
    return found


def detect_bazel_drift(module_text: str, lock_text: str) -> list[tuple[str, str, str]]:
    """Return (module, declared_version, locked_version) where they disagree.

    A declared version in MODULE.bazel that the lockfile does not record is
    lockfile drift: the lock was not refreshed after the declaration changed.
    """
    declared = bazel_mod_deps(module_text)
    locked = bazel_lock_versions(lock_text)
    drift = []
    for name, want in sorted(declared.items()):
        have = locked.get(name)
        if have is None or have != want:
            drift.append((name, want, have or "(absent)"))
    return drift


@dataclass
class Probe:
    """Why cargo did or did not accept a candidate version.

    `msrv` and `constraint` are the two reasons a lockfile update moves a crate
    backwards. They are different facts and are reported separately: a version
    can be unselectable purely because a dependent crate pins a tighter range,
    with no Rust requirement involved at all.
    """

    kind: str
    detail: str | None = None


def classify_cargo_output(output: str) -> Probe:
    """Classify combined cargo stdout/stderr from a probe run.

    Split out from `probe_crate_version` so the mapping can be tested against
    real cargo output without paying for a resolution on every test run.
    """
    msrv = re.search(r"requires Rust ([0-9][0-9.]*)", output)
    if msrv:
        return Probe("msrv", msrv.group(1))

    # "required by package `wasip2 v1.0.1+wasi-0.2.4`"
    blocked = re.search(r"required by package `([^`]+)`", output)
    if blocked and "failed to select a version" in output:
        return Probe("constraint", blocked.group(1))

    if "error" in output.lower():
        return Probe("unknown")

    return Probe("accepted")


def probe_crate_version(crate: str, version: str, cargo_dir: Path) -> Probe:
    """Ask cargo why it would not keep `version` of `crate`.

    Runs `cargo update -p <crate> --precise <version> --dry-run`, which does not
    touch the lockfile, and classifies the response:

    - `msrv`: cargo names a Rust floor, e.g. "requires Rust 1.87".
    - `constraint`: a dependent crate pins a range that excludes `version`.
    - `accepted`: cargo had no objection.

    This is evidence, not a verdict. A version cargo accepts is not thereby safe,
    and one it rejects may become usable if the workspace MSRV is raised or the
    dependent constraint is relaxed.
    """
    try:
        proc = subprocess.run(
            ["cargo", "update", "-p", crate, "--precise", version, "--dry-run"],
            cwd=cargo_dir,
            capture_output=True,
            text=True,
            timeout=600,
        )
    except (OSError, subprocess.TimeoutExpired):
        return Probe("unknown")

    return classify_cargo_output(proc.stdout + proc.stderr)




def format_summary(
    buckets: dict[str, list[PackageChange]],
    drift: list[tuple[str, str, str]],
    probes: dict[tuple[str, str], Probe],
    max_rows: int,
) -> str:
    lines: list[str] = ["## Lockfile changes", ""]
    lines.append(
        "Generated by `scripts/lockfile_summary.py`. The pull request body is "
        "maintained separately and is not overwritten."
    )
    lines.append("")

    if not any(buckets.values()) and not drift:
        lines.append("No lockfile changes detected.")
        return "\n".join(lines) + "\n"

    def section(title: str, items: list[str]) -> None:
        if not items:
            return
        lines.append(f"### {title} ({len(items)})")
        lines.append("")
        lines.extend(items[:max_rows])
        if len(items) > max_rows:
            lines.append(f"- _...and {len(items) - max_rows} more_")
        lines.append("")

    def versions(v: set[str]) -> str:
        return ", ".join(f"`{x}`" for x in sorted(v, key=Version))

    section("Crates added", [f"- `{c.name}` {versions(c.after)}" for c in buckets["added"]])
    section("Crates removed", [f"- `{c.name}`" for c in buckets["removed"]])
    section(
        "Upgraded",
        [f"- `{c.name}` {o} -> {n}" for c in buckets["upgraded"] for o, n in c.upgrades],
    )

    def describe_probe(p: Probe | None) -> str:
        if p is None:
            return ""
        if p.kind == "msrv":
            return f" — candidate requires Rust {p.detail}"
        if p.kind == "constraint":
            return f" — candidate blocked by `{p.detail}`"
        if p.kind == "accepted":
            return " — cargo accepted the candidate; cause not identified"
        return " — cargo gave no usable reason"

    downgrades = []
    for c in buckets["downgraded"]:
        for old, new in c.downgrades:
            note = describe_probe(probes.get((c.name, old)))
            downgrades.append(f"- `{c.name}` {old} -> {new}{note}")
    section("Downgraded", downgrades)

    def describe_other(c: PackageChange) -> str:
        parts = []
        if c.added:
            parts.append(f"added {versions(c.added)}")
        if c.removed:
            parts.append(f"removed {versions(c.removed)}")
        unchanged = c.before & c.after
        if unchanged:
            parts.append(f"unchanged {versions(unchanged)}")
        return f"- `{c.name}`: " + ", ".join(parts)

    section(
        "Other version changes",
        [describe_other(c) for c in buckets["version_lines"]],
    )

    if drift:
        lines.append("### MODULE.bazel.lock drift")
        lines.append("")
        lines.append("Declared in `MODULE.bazel` but not matched by the lockfile:")
        lines.append("")
        for module, declared, locked in drift:
            lines.append(f"- `{module}`: declared `{declared}`, locked `{locked}`")
        lines.append("")

    lines.append(
        "_Counts and version transitions are derived from the lockfiles. "
        "Whether any individual change is desirable is a review decision._"
    )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-ref", required=True, help="git ref to compare against")
    parser.add_argument(
        "--base-lock",
        help="path to the baseline Cargo.lock; defaults to reading it from --base-ref",
    )
    parser.add_argument("--manifest-dir", default=".", help="repository root")
    parser.add_argument(
        "--cargo-dir",
        help="directory containing Cargo.toml for MSRV probes; defaults to --manifest-dir",
    )
    parser.add_argument("--max-rows", type=int, default=40)
    parser.add_argument("--output", help="write summary here instead of stdout")
    parser.add_argument(
        "--msrv-workers",
        type=int,
        default=6,
        help="parallel cargo probes for downgraded crates (default: 6)",
    )
    parser.add_argument(
        "--no-probe",
        action="store_true",
        help="skip cargo probes; downgrades are listed without evidence",
    )
    args = parser.parse_args(argv)

    root = Path(args.manifest_dir).resolve()
    cargo_root = Path(args.cargo_dir).resolve() if args.cargo_dir else root

    def read(ref_path: str) -> str:
        return subprocess.run(
            ["git", "show", f"{args.base_ref}:{ref_path}"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout

    if args.base_lock:
        before = parse_cargo_lock(Path(args.base_lock).read_text(encoding="utf-8"))
    else:
        before = parse_cargo_lock(read("Cargo.lock"))
    after = parse_cargo_lock((root / "Cargo.lock").read_text(encoding="utf-8"))

    buckets = categorize(diff_cargo_locks(before, after))

    drift: list[tuple[str, str, str]] = []
    if (root / "MODULE.bazel").exists() and (root / "MODULE.bazel.lock").exists():
        drift = detect_bazel_drift(
            (root / "MODULE.bazel").read_text(encoding="utf-8"),
            (root / "MODULE.bazel.lock").read_text(encoding="utf-8"),
        )

    # Every downgrade is probed, because a crate that moved backwards was
    # blocked by something: either its own MSRV or a dependent's version
    # requirement. Probing is bounded because each call re-resolves the graph.
    probes: dict[tuple[str, str], Probe] = {}
    if not args.no_probe:
        targets = [
            (c.name, old) for c in buckets["downgraded"] for old, _ in c.downgrades
        ]
        if targets:
            workers = max(1, min(args.msrv_workers, len(targets)))
            with ThreadPoolExecutor(max_workers=workers) as pool:
                futures = {
                    pool.submit(probe_crate_version, name, ver, cargo_root): (name, ver)
                    for name, ver in targets
                }
                for future in as_completed(futures):
                    key = futures[future]
                    try:
                        probes[key] = future.result()
                    except Exception:  # pragma: no cover - defensive
                        probes[key] = Probe("unknown")

    summary = format_summary(buckets, drift, probes, args.max_rows)

    if args.output:
        Path(args.output).write_text(summary, encoding="utf-8")
    else:
        sys.stdout.write(summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
