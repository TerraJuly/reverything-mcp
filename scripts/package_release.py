"""Build a Windows x64 ZIP, verify bundled ES, and include dependency licenses.

Usage: python scripts/package_release.py
Requires Rust/Cargo and Python 3.10+. No Python packages required.
"""

import hashlib
import json
import pathlib
import struct
import subprocess
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"


def cargo(*args):
    return subprocess.run(
        ["cargo", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def main():
    manifest = json.loads((ROOT / "bin/es-manifest.json").read_text(encoding="utf-8"))
    binary = (ROOT / "bin/es.exe").read_bytes()
    if hashlib.sha256(binary).hexdigest() != manifest["exe_sha256"]:
        raise RuntimeError("Bundled ES hash mismatch; do not package an unverified executable")
    pe_offset = struct.unpack_from("<I", binary, 0x3C)[0]
    if binary[:2] != b"MZ" or binary[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise RuntimeError("Bundled ES is not a PE executable")
    if struct.unpack_from("<H", binary, pe_offset + 4)[0] != 0x8664:
        raise RuntimeError("Bundled ES is not x64")

    print("Building Windows x64 release...")
    cargo("build", "--release", "--locked", "--target", TARGET)
    metadata = json.loads(
        cargo("metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET)
    )
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root_id = metadata["resolve"]["root"]
    root_package = packages[root_id]
    reachable = set()

    def visit(package_id):
        if package_id in reachable:
            return
        reachable.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                visit(dependency["pkg"])

    visit(root_id)
    license_files = []
    dependency_notices = []
    for package_id in sorted(reachable):
        if package_id == root_id:
            continue
        package = packages[package_id]
        directory = pathlib.Path(package["manifest_path"]).parent
        notices = [
            path
            for path in directory.iterdir()
            if path.is_file()
            and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE"))
        ]
        if package.get("license_file"):
            notices.append(directory / package["license_file"])
        notices = sorted(set(notices))
        if not notices:
            raise RuntimeError(
                f"No bundled license text found for {package['name']} {package['version']}; "
                "review its license before distributing"
            )
        destination = f"licenses/rust/{package['name']}-{package['version']}"
        for path in notices:
            license_files.append((path, f"{destination}/{path.name}"))
        dependency_notices.append(
            f"{package['name']} {package['version']}\n"
            f"  License: {package.get('license') or 'see license file'}\n"
            f"  Source: {package.get('repository') or package.get('source', '')}\n"
            f"  Notices: {destination}/\n"
        )

    version = root_package["version"]
    bundle = f"reverything-mcp-{version}-windows-x64"
    destination = ROOT / "dist" / f"{bundle}.zip"
    destination.parent.mkdir(exist_ok=True)
    if destination.exists():
        raise FileExistsError(f"Release ZIP already exists: {destination}; inspect it before replacing")
    files = {
        "reverything-mcp.exe": ROOT / "target" / TARGET / "release/reverything-mcp.exe",
        "bin/es.exe": ROOT / "bin/es.exe",
        "bin/es-manifest.json": ROOT / "bin/es-manifest.json",
        "LICENSE": ROOT / "LICENSE",
        "licenses/ES-LICENSE.txt": ROOT / "licenses/ES-LICENSE.txt",
        "THIRD-PARTY-NOTICES.txt": ROOT / "THIRD-PARTY-NOTICES.txt",
        "README.md": ROOT / "README.md",
        "docs/search-syntax.md": ROOT / "docs/search-syntax.md",
    }
    with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, source in files.items():
            archive.write(source, f"{bundle}/{name}")
        for source, name in license_files:
            archive.write(source, f"{bundle}/{name}")
        archive.writestr(
            f"{bundle}/licenses/RUST-DEPENDENCIES.txt", "\n".join(dependency_notices)
        )
    print(f"Created {destination}")
    print(f"Included ES license and notices for {len(dependency_notices)} Rust dependencies")
    print("ZIP SHA256:", hashlib.sha256(destination.read_bytes()).hexdigest())


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(error.stdout)
        print(error.stderr)
        raise SystemExit(error.returncode) from error
