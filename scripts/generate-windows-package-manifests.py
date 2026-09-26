#!/usr/bin/env python3

import argparse
import json
import pathlib
import re


PACKAGE_IDENTIFIER = "coccinella-labs.Harper"
PACKAGE_NAME = "Harper"
PUBLISHER = "coccinella-labs"
HOMEPAGE = "https://github.com/coccinella-labs/harper"
LICENSE = "MIT OR Apache-2.0"
DESCRIPTION = "Terminal assistant for code and shell work."


def version_from_tag(release_tag: str) -> str:
    prefix = "harper-"
    if not release_tag.startswith(prefix):
        raise ValueError(f"release tag must start with '{prefix}': {release_tag}")
    version = release_tag[len(prefix) :]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?", version):
        raise ValueError(f"release tag does not contain a package version: {release_tag}")
    return version


def default_windows_asset_url(release_tag: str) -> str:
    return (
        f"https://github.com/coccinella-labs/harper/releases/download/"
        f"{release_tag}/harper-windows-x86_64.zip"
    )


def write_scoop_manifest(output_dir: pathlib.Path, version: str, asset_url: str, sha256: str) -> None:
    autoupdate_url = (
        "https://github.com/coccinella-labs/harper/releases/download/"
        "harper-$version/harper-windows-x86_64.zip"
    )
    manifest = {
        "version": version,
        "description": DESCRIPTION,
        "homepage": HOMEPAGE,
        "license": LICENSE,
        "architecture": {
            "64bit": {
                "url": asset_url,
                "hash": sha256,
            }
        },
        "bin": "harper.exe",
        "checkver": {
            "github": HOMEPAGE,
            "regex": r"harper-([\d.]+)",
        },
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": autoupdate_url,
                }
            }
        },
    }

    path = output_dir / "scoop" / "harper.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def write_winget_manifests(output_dir: pathlib.Path, version: str, asset_url: str, sha256: str) -> None:
    manifest_dir = (
        output_dir
        / "winget"
        / "manifests"
        / "h"
        / PUBLISHER
        / PACKAGE_NAME
        / version
    )
    manifest_dir.mkdir(parents=True, exist_ok=True)

    version_manifest = f"""PackageIdentifier: {PACKAGE_IDENTIFIER}
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.10.0
"""
    locale_manifest = f"""PackageIdentifier: {PACKAGE_IDENTIFIER}
PackageVersion: {version}
PackageLocale: en-US
Publisher: {PUBLISHER}
PackageName: {PACKAGE_NAME}
License: {LICENSE}
ShortDescription: {DESCRIPTION}
PackageUrl: {HOMEPAGE}
ManifestType: defaultLocale
ManifestVersion: 1.10.0
"""
    installer_manifest = f"""PackageIdentifier: {PACKAGE_IDENTIFIER}
PackageVersion: {version}
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: harper.exe
    PortableCommandAlias: harper
Installers:
  - Architecture: x64
    InstallerUrl: {asset_url}
    InstallerSha256: {sha256.upper()}
ManifestType: installer
ManifestVersion: 1.10.0
"""

    (manifest_dir / f"{PACKAGE_IDENTIFIER}.yaml").write_text(version_manifest, encoding="utf-8")
    (manifest_dir / f"{PACKAGE_IDENTIFIER}.locale.en-US.yaml").write_text(
        locale_manifest, encoding="utf-8"
    )
    (manifest_dir / f"{PACKAGE_IDENTIFIER}.installer.yaml").write_text(
        installer_manifest, encoding="utf-8"
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate Windows package-manager manifests for a Harper release."
    )
    parser.add_argument("--release-tag", required=True, help="Release tag like harper-0.20.1")
    parser.add_argument("--sha256", required=True, help="SHA256 of harper-windows-x86_64.zip")
    parser.add_argument("--asset-url", help="Override the Windows release asset URL")
    parser.add_argument("--output-dir", required=True, help="Directory to write manifests into")
    args = parser.parse_args()

    sha256 = args.sha256.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", sha256):
        raise SystemExit("sha256 must be 64 lowercase or uppercase hexadecimal characters")

    try:
        version = version_from_tag(args.release_tag)
    except ValueError as err:
        raise SystemExit(str(err)) from err

    asset_url = args.asset_url or default_windows_asset_url(args.release_tag)
    output_dir = pathlib.Path(args.output_dir)
    write_scoop_manifest(output_dir, version, asset_url, sha256)
    write_winget_manifests(output_dir, version, asset_url, sha256)
    print(f"generated Windows package manifests for {args.release_tag} in {output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
