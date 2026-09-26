#!/usr/bin/env python3

import argparse
import os
import pathlib
import re
import shutil
import subprocess
import tarfile


PACKAGE_NAME = "harper"
MAINTAINER = "coccinella-labs <coccinella.labs@icloud.com>"
HOMEPAGE = "https://github.com/coccinella-labs/harper"
DESCRIPTION = "Terminal assistant for code and shell work."


ARCHIVES = {
    "amd64": "harper-linux-x86_64.tar.gz",
    "arm64": "harper-linux-aarch64.tar.gz",
}


def version_from_tag(release_tag: str) -> str:
    prefix = "harper-"
    if not release_tag.startswith(prefix):
        raise ValueError(f"release tag must start with '{prefix}': {release_tag}")
    version = release_tag[len(prefix) :]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+~][0-9A-Za-z.]+)?", version):
        raise ValueError(f"release tag does not contain a Debian-compatible version: {release_tag}")
    return version


def extract_binary(archive_path: pathlib.Path, work_dir: pathlib.Path) -> pathlib.Path:
    extract_dir = work_dir / "extract"
    extract_dir.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive_path, "r:gz") as archive:
        archive.extractall(extract_dir, filter="data")

    matches = [path for path in extract_dir.rglob("harper") if path.is_file()]
    if len(matches) != 1:
        raise ValueError(f"expected one harper binary in {archive_path}, found {len(matches)}")
    return matches[0]


def installed_size_kib(path: pathlib.Path) -> int:
    return max(1, (path.stat().st_size + 1023) // 1024)


def build_deb(archive_path: pathlib.Path, output_dir: pathlib.Path, version: str, arch: str) -> pathlib.Path:
    if shutil.which("dpkg-deb") is None:
        raise RuntimeError("dpkg-deb is required to build Debian packages")

    package_root = output_dir / "work" / arch / f"{PACKAGE_NAME}_{version}_{arch}"
    debian_dir = package_root / "DEBIAN"
    bin_dir = package_root / "usr" / "bin"
    debian_dir.mkdir(parents=True, exist_ok=True)
    bin_dir.mkdir(parents=True, exist_ok=True)

    binary = extract_binary(archive_path, output_dir / "work" / arch)
    target_binary = bin_dir / "harper"
    shutil.copy2(binary, target_binary)
    target_binary.chmod(0o755)

    control = f"""Package: {PACKAGE_NAME}
Version: {version}
Section: utils
Priority: optional
Architecture: {arch}
Maintainer: {MAINTAINER}
Installed-Size: {installed_size_kib(target_binary)}
Homepage: {HOMEPAGE}
Description: {DESCRIPTION}
"""
    (debian_dir / "control").write_text(control, encoding="utf-8")

    output_path = output_dir / f"{PACKAGE_NAME}_{version}_{arch}.deb"
    subprocess.run(
        ["dpkg-deb", "--build", "--root-owner-group", str(package_root), str(output_path)],
        check=True,
    )
    return output_path


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate Linux native packages for Harper.")
    parser.add_argument("--release-tag", required=True, help="Release tag like harper-0.20.1")
    parser.add_argument("--artifacts-dir", required=True, help="Directory containing Linux release tarballs")
    parser.add_argument("--output-dir", required=True, help="Directory to write packages into")
    parser.add_argument(
        "--arch",
        action="append",
        choices=sorted(ARCHIVES),
        help="Architecture to package. Defaults to all supported architectures.",
    )
    args = parser.parse_args()

    try:
        version = version_from_tag(args.release_tag)
    except ValueError as err:
        raise SystemExit(str(err)) from err

    artifacts_dir = pathlib.Path(args.artifacts_dir)
    output_dir = pathlib.Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    arches = args.arch or sorted(ARCHIVES)

    for arch in arches:
        archive_path = artifacts_dir / ARCHIVES[arch]
        if not archive_path.is_file():
            raise SystemExit(f"missing release artifact: {archive_path}")
        try:
            package_path = build_deb(archive_path, output_dir, version, arch)
        except RuntimeError as err:
            raise SystemExit(str(err)) from err
        print(f"generated {package_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
