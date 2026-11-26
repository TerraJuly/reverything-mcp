"""Validate a v-prefixed SemVer tag against Cargo.toml via Cargo metadata.

Usage: python scripts/check_release_tag.py v0.1.0
In Actions, RELEASE_TAG supplies the tag and GITHUB_OUTPUT receives outputs.
Only version strings are logged, never file paths or file metadata.
"""

import argparse
import json
import os
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
TAG_PATTERN = re.compile(
    r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def validate_tag(tag, package_version):
    if not TAG_PATTERN.fullmatch(tag):
        raise ValueError("Release tag must be v-prefixed SemVer, e.g. v0.1.0 or v0.2.0-rc.1")
    version = tag[1:]
    prerelease = version.partition("+")[0].partition("-")[2]
    if any(part.isdigit() and len(part) > 1 and part.startswith("0") for part in prerelease.split(".")):
        raise ValueError("Numeric prerelease identifiers must not have leading zeros")
    if version != package_version:
        raise ValueError("Release tag does not match the version in Cargo.toml")
    return {
        "version": version,
        "prerelease": "true" if prerelease else "false",
        "bundle_name": f"reverything-mcp-{version}-windows-x64",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag", nargs="?", default=os.environ.get("RELEASE_TAG"))
    options = parser.parse_args()
    if options.tag is None:
        parser.error("provide a tag argument or set RELEASE_TAG")
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
    )
    if result.returncode:
        raise RuntimeError("Cargo metadata failed; check Cargo.toml and Cargo.lock")
    metadata = json.loads(result.stdout)
    manifest = (ROOT / "Cargo.toml").resolve()
    package = next(
        item for item in metadata["packages"]
        if pathlib.Path(item["manifest_path"]).resolve() == manifest
    )
    outputs = validate_tag(options.tag, package["version"])
    if destination := os.environ.get("GITHUB_OUTPUT"):
        with pathlib.Path(destination).open("a", encoding="utf-8") as output:
            for key, value in outputs.items():
                output.write(f"{key}={value}\n")
    print(f"Validated version: {outputs['version']}; prerelease: {outputs['prerelease']}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError) as error:
        raise SystemExit(str(error)) from None
