#!/usr/bin/env python3
"""Fetch a pinned, isolated sfizz + original Salamander piano runtime on Linux.

Requires x86_64 Ubuntu 22.04-compatible libraries, Python 3, and dpkg-deb.
No packages are installed system-wide. Samples: Alexander Holm, CC BY 3.0.
"""
from concurrent.futures import ThreadPoolExecutor
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
PIANO = ROOT / ".runtime/piano"
SFIZZ = ROOT / ".runtime/sfizz"
ARCHIVE_NAME = "SalamanderGrandPianoV3_44.1khz16bit.tar.bz2"
ARCHIVE_URL = "https://archive.org/download/SalamanderGrandPianoV3/" + ARCHIVE_NAME
ARCHIVE_SIZE = 488713261
ARCHIVE_SHA256 = "fe595c7722b70860e6377f82948d1c0cfcf27ecebf500dc79534408a78a62892"
PACKAGE_URL = "https://download.opensuse.org/repositories/home:/sfztools:/sfizz/xUbuntu_22.04/amd64/"
PACKAGES = {
    "libsfizz1_1.2.3-0_amd64.deb": "b5e7fa5e9e2c1f321ea58054ae25b8f3fb41a68666534b9f098ed6575e3774a2",
    "sfizz_1.2.3-0_amd64.deb": "aa421694b98ec167c712255b03ee268fe9901c1bc72972c83c4232f81331d43c",
}


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download(url, path, expected, size=None):
    if path.exists() and checksum(path) == expected:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    parts = path.with_name(path.name + ".parts")
    parts.mkdir(exist_ok=True)
    count = 12 if size else 1
    chunk = (size + count - 1) // count if size else None

    def fetch(index):
        target = parts / str(index)
        headers = {}
        start = end = None
        if size:
            start, end = index * chunk, min(size - 1, (index + 1) * chunk - 1)
            headers["Range"] = f"bytes={start}-{end}"
        for retry in range(4):
            try:
                request = urllib.request.Request(url, headers=headers)
                with urllib.request.urlopen(request, timeout=90) as source, target.open("wb") as output:
                    if size and (source.status != 206 or source.headers.get("Content-Range") != f"bytes {start}-{end}/{size}"):
                        raise ValueError("Server did not honor the requested byte range.")
                    shutil.copyfileobj(source, output, 1024 * 1024)
                if size and target.stat().st_size != end - start + 1:
                    raise ValueError("Incomplete download.")
                print(f"{path.name}: part {index + 1}/{count}", flush=True)
                return
            except Exception:
                if retry == 3:
                    raise
                time.sleep(retry + 2)

    with ThreadPoolExecutor(max_workers=count) as pool:
        list(pool.map(fetch, range(count)))
    pending = path.with_suffix(path.suffix + ".pending")
    with pending.open("wb") as output:
        for index in range(count):
            with (parts / str(index)).open("rb") as source:
                shutil.copyfileobj(source, output, 1024 * 1024)
    if checksum(pending) != expected:
        raise ValueError(f"Checksum mismatch for {path.name}; refusing installation.")
    pending.replace(path)
    shutil.rmtree(parts)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    if not shutil.which("dpkg-deb"):
        raise SystemExit("dpkg-deb is required to extract the isolated renderer.")
    for filename, digest in PACKAGES.items():
        package = SFIZZ / "packages" / filename
        download(PACKAGE_URL + filename, package, digest)
        subprocess.run(["dpkg-deb", "-x", str(package), str(SFIZZ / "root")], check=True)
    archive = PIANO / ARCHIVE_NAME
    download(ARCHIVE_URL, archive, ARCHIVE_SHA256, ARCHIVE_SIZE)
    with tarfile.open(archive, "r|bz2") as source:
        for item in source:
            (PIANO / item.name).resolve().relative_to(PIANO.resolve())
            if not (item.isdir() or item.isfile()):
                raise ValueError("Unexpected archive entry type.")
            source.extract(item, PIANO)
    record = {
        "name": "Salamander Grand Piano V3", "author": "Alexander Holm",
        "license": "CC BY 3.0", "license_url": "https://creativecommons.org/licenses/by/3.0/",
        "archive_url": ARCHIVE_URL, "archive_sha256": ARCHIVE_SHA256,
        "sample_format": "44100 Hz, 16-bit stereo PCM",
        "renderer": "sfizz 1.2.3", "packages": PACKAGES,
        "pianoteq": "Unavailable unless a licensed installation is supplied; no demo used.",
    }
    (PIANO / "provenance.json").write_text(json.dumps(record, indent=2) + "\n")
    env = dict(os.environ)
    env["LD_LIBRARY_PATH"] = str(SFIZZ / "root/usr/lib") + (
        ":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else ""
    )
    subprocess.run([str(SFIZZ / "root/usr/bin/sfizz_render"), "--help"], check=True, env=env)
    print("Ready: python3 research/listening/render_piano.py input.mid output.wav")


if __name__ == "__main__":
    main()
