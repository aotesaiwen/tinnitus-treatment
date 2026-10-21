#!/usr/bin/env python3
"""Download pinned official Stable Audio checkpoints and verify every LFS hash."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import time

import requests

ROOT = Path(__file__).resolve().parents[2]
DESTINATION = ROOT / ".runtime/stable-audio-checkpoints"
REVISIONS = {
    "medium": "d4d3579395a8ac0df4ab13b5b1d7138ce7290f81",
    "small-music": "8bdfb53c6b0fbc62113cd571fd37373efe15fa14",
}


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(8 * 1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def download_file(session, name, revision, entry):
    relative = entry["path"]
    target = DESTINATION / name / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    expected_size = entry["size"]
    expected_hash = (entry.get("lfs") or {}).get("sha256")
    if target.exists() and target.stat().st_size == expected_size:
        actual = digest(target)
        if not expected_hash or actual == expected_hash:
            return {"path": relative, "bytes": expected_size, "sha256": actual}
    partial = target.with_suffix(target.suffix + ".partial")
    url = f"https://huggingface.co/stabilityai/stable-audio-3-{name}/resolve/{revision}/{relative}"
    for attempt in range(4):
        offset = partial.stat().st_size if partial.exists() else 0
        if offset > expected_size:
            raise ValueError(f"Oversized partial file: {name}/{relative}")
        try:
            if offset < expected_size:
                headers = {"Range": f"bytes={offset}-"} if offset else {}
                with session.get(url, headers=headers, stream=True, timeout=(20, 60)) as response:
                    if response.status_code not in (200, 206):
                        raise RuntimeError(f"HTTP {response.status_code}: {name}/{relative}")
                    if response.status_code == 206:
                        if not response.headers.get("Content-Range", "").startswith(f"bytes {offset}-"):
                            raise ValueError("Server returned an unexpected byte range")
                    elif offset:
                        offset = 0
                    print(f"Downloading {name}/{relative} from {offset:,} / {expected_size:,} bytes", flush=True)
                    with partial.open("ab" if offset else "wb") as output:
                        previous = offset // (512 * 1024 * 1024)
                        for block in response.iter_content(1024 * 1024):
                            output.write(block)
                            offset += len(block)
                            if offset > expected_size:
                                raise ValueError("Download exceeds expected file size")
                            progress = offset // (512 * 1024 * 1024)
                            if progress > previous:
                                print(f"{name}/{relative}: {offset / expected_size:.0%}", flush=True)
                                previous = progress
            if partial.stat().st_size != expected_size:
                raise OSError("Incomplete file")
            actual = digest(partial)
            if expected_hash and actual != expected_hash:
                raise ValueError(f"SHA-256 mismatch: {name}/{relative}")
            partial.replace(target)
            print(f"Verified {name}/{relative}", flush=True)
            return {"path": relative, "bytes": expected_size, "sha256": actual}
        except (requests.RequestException, OSError) as error:
            # Never print authenticated request objects, signed URLs, or headers.
            print(f"Retry {attempt + 1}: {name}/{relative}: {type(error).__name__}", flush=True)
            if attempt == 3:
                raise RuntimeError(f"Download failed: {name}/{relative}") from None
            time.sleep(2)
    raise RuntimeError("Download did not complete")


def setup_model(name, token, proxy):
    session = requests.Session()
    session.trust_env = False
    if proxy:
        session.proxies = {"https": proxy}
    session.headers["Authorization"] = "Bearer " + token
    revision = REVISIONS[name]
    repo = "stabilityai/stable-audio-3-" + name
    response = session.get(f"https://huggingface.co/api/models/{repo}/revision/{revision}",
                           params={"blobs": "true"}, timeout=30)
    if response.status_code != 200:
        raise RuntimeError(f"Model access failed: {name}, HTTP {response.status_code}")
    data = response.json()
    if data["sha"] != revision:
        raise ValueError("Unexpected model revision")
    # Download all runtime files and accompanying license texts, excluding artwork.
    selected = [f for f in data["siblings"] if f["rfilename"].endswith(
        (".json", ".safetensors", ".model", ".md")) or f["rfilename"] == "NOTICE"]
    destination = DESTINATION / name
    destination.mkdir(parents=True, exist_ok=True)
    metadata = {"repo_id": repo, "revision": revision, "files": [
        {"path": f["rfilename"], "size": f["size"], "lfs": f.get("lfs")} for f in selected]}
    (destination / "provenance.json").write_text(json.dumps(metadata, indent=2) + "\n")
    verified = [download_file(session, name, revision, entry) for entry in metadata["files"]]
    (destination / "verified.json").write_text(json.dumps({
        "repo_id": repo, "revision": revision, "files": verified}, indent=2) + "\n")
    return {"model": name, "files": len(verified), "bytes": sum(f["bytes"] for f in verified)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", nargs="+", choices=REVISIONS, default=["small-music", "medium"])
    parser.add_argument("--proxy", default=os.environ.get("MUSIC_DOWNLOAD_PROXY", ""))
    args = parser.parse_args()
    path = ROOT / ".runtime/music-access/credentials.json"
    credentials = json.loads(path.read_text()) if path.exists() else {}
    token = os.environ.get("HF_TOKEN") or credentials.get("HF_TOKEN")
    if not token:
        raise SystemExit("Configure a Hugging Face read token first.")
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(setup_model, name, token, args.proxy) for name in dict.fromkeys(args.models)]
        for result in futures:
            print(json.dumps(result.result()), flush=True)


if __name__ == "__main__":
    main()
