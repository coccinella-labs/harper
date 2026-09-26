#!/usr/bin/env python3
"""Tests for scripts/lockfile_summary.py.

The multi-version cases matter most: the previous diff-scraping generator
paired versions by proximity in the diff text and mis-reported crates that
appear at several versions at once (`rand` is present at three). Each of those
cases is pinned here so the failure cannot come back.

Run: python3 scripts/test_lockfile_summary.py
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lockfile_summary import (  # noqa: E402
    PackageChange,
    Version,
    bazel_lock_versions,
    bazel_mod_deps,
    categorize,
    detect_bazel_drift,
    diff_cargo_locks,
    format_summary,
    parse_cargo_lock,
)


def lock(entries: list[tuple[str, str, bool]]) -> str:
    """Build a Cargo.lock body. Third element marks a registry (sourced) pkg."""
    out = ["version = 4", ""]
    for name, version, sourced in entries:
        out.append("[[package]]")
        out.append(f'name = "{name}"')
        out.append(f'version = "{version}"')
        if sourced:
            out.append('source = "registry+https://github.com/rust-lang/crates.io-index"')
        out.append("")
    return "\n".join(out)


class TestVersionOrdering(unittest.TestCase):
    def test_numeric_order(self) -> None:
        self.assertLess(Version("0.4.19"), Version("0.4.20"))
        self.assertLess(Version("0.9.5"), Version("0.10.3"))
        self.assertLess(Version("1.9.0"), Version("1.10.0"))

    def test_build_metadata_ignored(self) -> None:
        self.assertEqual(Version("1.1.2+spec-1.1.0"), Version("1.1.2"))
        self.assertLess(Version("1.1.2"), Version("1.1.3+spec-1.1.0"))

    def test_unparsable_sorts_lowest(self) -> None:
        self.assertLess(Version("not-a-version"), Version("0.0.1"))


class TestParseCargoLock(unittest.TestCase):
    def test_skips_path_dependencies(self) -> None:
        text = lock(
            [
                ("harper-core", "0.23.0", False),
                ("anyhow", "1.0.104", True),
            ]
        )
        parsed = parse_cargo_lock(text)
        self.assertEqual(parsed, {"anyhow": {"1.0.104"}})

    def test_collects_multiple_versions(self) -> None:
        text = lock(
            [
                ("rand", "0.8.8", True),
                ("rand", "0.9.5", True),
                ("rand", "0.10.3", True),
            ]
        )
        self.assertEqual(parse_cargo_lock(text)["rand"], {"0.8.8", "0.9.5", "0.10.3"})


class TestMultiVersionCrates(unittest.TestCase):
    """The regression that motivated replacing the diff scraper."""

    def test_rand_three_versions_pair_correctly(self) -> None:
        before = parse_cargo_lock(lock([("rand", "0.8.6", True), ("rand", "0.9.4", True), ("rand", "0.10.3", True)]))
        after = parse_cargo_lock(lock([("rand", "0.8.8", True), ("rand", "0.9.5", True), ("rand", "0.10.3", True)]))
        changes = diff_cargo_locks(before, after)
        self.assertEqual(len(changes), 1)
        change = changes[0]
        # Exactly the two lines that moved; 0.10.3 is untouched and must not
        # appear as a bump.
        self.assertEqual(change.added, {"0.8.8", "0.9.5"})
        self.assertEqual(change.removed, {"0.8.6", "0.9.4"})
        buckets = categorize(changes)
        # More than one version moved, so no single pairing is invented.
        self.assertEqual(buckets["upgraded"], [])
        self.assertEqual(buckets["downgraded"], [])

    def test_untouched_version_is_never_reported(self) -> None:
        before = parse_cargo_lock(lock([("rand", "0.8.6", True), ("rand", "0.9.4", True), ("rand", "0.10.3", True)]))
        after = parse_cargo_lock(lock([("rand", "0.8.8", True), ("rand", "0.9.5", True), ("rand", "0.10.3", True)]))
        rendered = format_summary(categorize(diff_cargo_locks(before, after)), [], {}, 50)
        self.assertNotIn("0.10.2", rendered)
        self.assertNotIn("0.10.3 ->", rendered)
        # The real transitions must be visible.
        self.assertIn("0.8.6", rendered)
        self.assertIn("0.9.4", rendered)


class TestCategorize(unittest.TestCase):
    def test_simple_upgrade(self) -> None:
        b = categorize(diff_cargo_locks({"a": {"1.0.0"}}, {"a": {"1.0.1"}}))
        self.assertEqual([c.name for c in b["upgraded"]], ["a"])

    def test_simple_downgrade(self) -> None:
        b = categorize(diff_cargo_locks({"a": {"1.0.1"}}, {"a": {"1.0.0"}}))
        self.assertEqual([c.name for c in b["downgraded"]], ["a"])

    def test_new_and_removed_crates(self) -> None:
        b = categorize(diff_cargo_locks({"gone": {"1.0.0"}}, {"new": {"1.0.0"}}))
        self.assertEqual([c.name for c in b["added"]], ["new"])
        self.assertEqual([c.name for c in b["removed"]], ["gone"])

    def test_added_version_line(self) -> None:
        b = categorize(
            diff_cargo_locks({"zune-jpeg": {"0.5.15"}}, {"zune-jpeg": {"0.4.21", "0.5.15"}})
        )
        self.assertEqual([c.name for c in b["version_lines"]], ["zune-jpeg"])
        self.assertEqual(b["upgraded"], [])

    def test_removed_version_line(self) -> None:
        b = categorize(
            diff_cargo_locks({"aes": {"0.8.4", "0.9.3"}}, {"aes": {"0.8.4"}})
        )
        self.assertEqual([c.name for c in b["version_lines"]], ["aes"])


class TestPackageChange(unittest.TestCase):
    def test_single_line_change_pairs_even_with_other_versions(self) -> None:
        # syn is present at two versions; one line moves. That is
        # unambiguous and should be reported as a transition.
        c = PackageChange(name="syn", before={"1.0.109", "2.0.117"}, after={"1.0.109", "2.0.119"})
        self.assertEqual(c.upgrades, [("2.0.117", "2.0.119")])
        self.assertEqual(c.downgrades, [])

    def test_no_pairing_when_multiple_lines_move(self) -> None:
        # rand moves two of its three versions: any single pairing is a guess.
        c = PackageChange(
            name="rand",
            before={"0.8.6", "0.9.4", "0.10.3"},
            after={"0.8.8", "0.9.5", "0.10.3"},
        )
        self.assertEqual(c.upgrades, [])
        self.assertEqual(c.downgrades, [])

    def test_equal_versions_are_not_a_change(self) -> None:
        self.assertEqual(diff_cargo_locks({"a": {"1.0.0"}}, {"a": {"1.0.0"}}), [])


class TestBazelDrift(unittest.TestCase):
    def test_parses_bazel_deps(self) -> None:
        # MODULE.bazel is Starlark, not TOML.
        text = 'bazel_dep(name = "rules_rust", version = "0.74.0")\n'
        self.assertEqual(bazel_mod_deps(text), {"rules_rust": "0.74.0"})

    def test_parses_multiple_and_multiline(self) -> None:
        text = (
            'bazel_dep(name = "rules_rust", version = "0.74.0")\n'
            'bazel_dep(\n'
            '    name = "platforms",\n'
            '    version = "1.1.0",\n'
            ')\n'
        )
        self.assertEqual(
            bazel_mod_deps(text), {"rules_rust": "0.74.0", "platforms": "1.1.0"}
        )

    def test_ignores_non_dep_matches(self) -> None:
        text = '# bazel_dep(name = "commented", version = "9.9.9")\n'
        self.assertEqual(bazel_mod_deps(text), {})

    def test_reads_locked_version_from_registry_urls(self) -> None:
        # Real MODULE.bazel.lock shape: the resolved module version appears in
        # registryFileHashes keys, not as a structured record.
        lock_text = json.dumps(
            {
                "lockFileVersion": 26,
                "registryFileHashes": {
                    "https://bcr.bazel.build/modules/rules_rust/0.69.0/MODULE.bazel": "abc",
                    "https://bcr.bazel.build/modules/rules_rust/0.69.0/source.json": "def",
                    "https://bcr.bazel.build/modules/platforms/1.0.0/MODULE.bazel": "ghi",
                },
            }
        )
        found = bazel_lock_versions(lock_text)
        self.assertEqual(found["rules_rust"], "0.69.0")
        self.assertEqual(found["platforms"], "1.0.0")

    def test_reads_locked_version(self) -> None:
        # MODULE.bazel.lock is JSON, not TOML.
        lock_text = json.dumps(
            {
                "moduleExtensions": {
                    "@@rules_rust+//:extensions.bzl%dev": {
                        "devDependency": [{"name": "rules_rust", "version": "0.69.0"}]
                    }
                }
            }
        )
        self.assertEqual(bazel_lock_versions(lock_text)["rules_rust"], "0.69.0")

    def test_detects_drift(self) -> None:
        module = 'bazel_dep(name = "rules_rust", version = "0.74.0")\n'
        lock_text = json.dumps(
            {
                "moduleExtensions": {
                    "@@rules_rust+//:extensions.bzl%dev": {
                        "devDependency": [{"name": "rules_rust", "version": "0.69.0"}]
                    }
                }
            }
        )
        self.assertEqual(
            detect_bazel_drift(module, lock_text),
            [("rules_rust", "0.74.0", "0.69.0")],
        )

    def test_no_drift_when_matched(self) -> None:
        module = 'bazel_dep(name = "rules_rust", version = "0.74.0")\n'
        lock_text = json.dumps(
            {
                "moduleExtensions": {
                    "@@rules_rust+//:extensions.bzl%dev": {
                        "devDependency": [{"name": "rules_rust", "version": "0.74.0"}]
                    }
                }
            }
        )
        self.assertEqual(detect_bazel_drift(module, lock_text), [])

    def test_absent_module_is_drift(self) -> None:
        module = 'bazel_dep(name = "rules_rust", version = "0.74.0")\n'
        self.assertEqual(
            detect_bazel_drift(module, "{}"), [("rules_rust", "0.74.0", "(absent)")]
        )

    def test_malformed_lock_is_not_fatal(self) -> None:
        self.assertEqual(bazel_lock_versions("not json"), {})


class TestSummaryText(unittest.TestCase):
    def test_no_editorial_verdicts(self) -> None:
        """The summary states facts; it must not editorialize."""
        rendered = format_summary(
            categorize(diff_cargo_locks({"zbus": {"5.19.0"}}, {"zbus": {"5.13.2"}})),
            [],
            {("zbus", "5.19.0"): "1.87"},
            50,
        )
        lowered = rendered.lower()
        for banned in ("expected", "safe", "regression", "should be", "intentional"):
            self.assertNotIn(banned, lowered, f"summary must not assert {banned!r}")

    def test_reports_msrv_as_fact(self) -> None:
        rendered = format_summary(
            categorize(diff_cargo_locks({"zbus": {"5.19.0"}}, {"zbus": {"5.13.2"}})),
            [],
            {("zbus", "5.19.0"): "1.87"},
            50,
        )
        self.assertIn("requires Rust 1.87", rendered)

    def test_empty_diff(self) -> None:
        rendered = format_summary(categorize([]), [], {}, 50)
        self.assertIn("No lockfile changes detected", rendered)

    def test_truncates_long_lists(self) -> None:
        before = {f"c{i}": {"1.0.0"} for i in range(60)}
        after = {f"c{i}": {"1.0.1"} for i in range(60)}
        rendered = format_summary(categorize(diff_cargo_locks(before, after)), [], {}, 10)
        self.assertIn("and 50 more", rendered)


if __name__ == "__main__":
    unittest.main(verbosity=2)
