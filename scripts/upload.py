#!/usr/bin/env python3
"""Upload the signed IPAs from a GitHub release to App Store Connect (TestFlight).

    upload.py <release-tag> [app ids...]

Uses the App Store Connect build upload API (buildUploads / buildUploadFiles), so it runs anywhere
Python does. Env: ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH. Needs `gh` for the release assets.
Apps without an App Store Connect record, and builds already uploaded, are skipped.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time

import requests

sys.path.insert(0, os.path.dirname(__file__))
import asc  # noqa: E402

REPO = "keykeyg/craft-ios"


def release_assets(tag):
    out = subprocess.run(["gh", "api", f"repos/{REPO}/releases/tags/{tag}"], capture_output=True, text=True, check=True).stdout
    return {a["name"]: a for a in json.loads(out)["assets"]}


def download(asset, dest):
    r = subprocess.run(["gh", "api", "-H", "Accept: application/octet-stream", f"repos/{REPO}/releases/assets/{asset['id']}"],
                       capture_output=True, check=True)
    open(dest, "wb").write(r.stdout)
    return dest


def already_uploaded(app_id, version, build):
    j = asc.call("GET", "/v1/builds", params={"filter[app]": app_id, "filter[version]": build,
                                              "filter[preReleaseVersion.version]": version, "limit": 1})
    if j["data"]:
        return True
    ups = asc.call("GET", f"/v1/apps/{app_id}/buildUploads", params={"limit": 50})
    return any(u["attributes"].get("cfBundleVersion") == build and u["attributes"].get("cfBundleShortVersionString") == version
               and (u["attributes"].get("state") or {}) != "FAILED" for u in ups.get("data", []))


def upload(app_id, ipa, version, build):
    size = os.path.getsize(ipa)
    data = open(ipa, "rb").read()
    up = asc.call("POST", "/v1/buildUploads", {"data": {"type": "buildUploads",
        "attributes": {"cfBundleShortVersionString": version, "cfBundleVersion": build, "platform": "IOS"},
        "relationships": {"app": {"data": {"type": "apps", "id": app_id}}}}})["data"]
    f = asc.call("POST", "/v1/buildUploadFiles", {"data": {"type": "buildUploadFiles",
        "attributes": {"assetType": "ASSET", "fileName": os.path.basename(ipa), "fileSize": size, "uti": "com.apple.ipa"},
        "relationships": {"buildUpload": {"data": {"type": "buildUploads", "id": up["id"]}}}}})["data"]
    ops = f["attributes"]["uploadOperations"]
    for op in ops:
        chunk = data[op["offset"]: op["offset"] + op["length"]]
        headers = {h["name"]: h["value"] for h in op.get("requestHeaders") or []}
        for attempt in range(4):
            r = requests.request(op["method"], op["url"], data=chunk, headers=headers, timeout=600)
            if r.status_code < 300:
                break
            time.sleep(5 * (attempt + 1))
        else:
            raise SystemExit(f"chunk at {op['offset']} failed: {r.status_code} {r.text[:300]}")
    asc.call("PATCH", f"/v1/buildUploadFiles/{f['id']}", {"data": {"type": "buildUploadFiles", "id": f["id"],
        "attributes": {"uploaded": True,
                       "sourceFileChecksums": {"file": {"hash": hashlib.md5(data).hexdigest(), "algorithm": "MD5"}}}}})
    print(f"  sent {size / 1e6:.1f} MB in {len(ops)} part(s); waiting for App Store Connect")
    for _ in range(90):
        a = asc.call("GET", f"/v1/buildUploads/{up['id']}")["data"]["attributes"]
        state = a.get("state")
        if isinstance(state, dict):
            st = state.get("state")
            details = state.get("errors") or state.get("warnings") or []
        else:
            st, details = state, []
        if st in ("COMPLETE", "FAILED"):
            print(f"  upload {st}")
            for d in details:
                print("   ", d.get("code"), d.get("description") or d)
            return st == "COMPLETE"
        time.sleep(10)
    print("  still processing; check again later")
    return True


def main():
    tag, only = sys.argv[1], set(sys.argv[2:])
    assets = release_assets(tag)
    apps = {a["attributes"]["bundleId"]: a for a in asc.get_all("/v1/apps", {"fields[apps]": "name,bundleId"})}
    tmp = tempfile.mkdtemp()
    for name in sorted(assets):
        if not name.endswith(".json"):
            continue
        meta = json.loads(open(download(assets[name], os.path.join(tmp, name))).read())
        if only and meta["id"] not in only:
            continue
        app = apps.get(meta["bundle"])
        label = f"{meta['id']} {meta['version']} ({meta['build']})"
        if not meta.get("signed") or meta["id"] + ".ipa" not in assets:
            print(f"{label}: no signed IPA in {tag}")
            continue
        if app is None:
            print(f"{label}: no App Store Connect record for {meta['bundle']} yet, skipped")
            continue
        if already_uploaded(app["id"], meta["version"], meta["build"]):
            print(f"{label}: already uploaded")
            continue
        print(f"{label}: uploading to {app['attributes']['name']}")
        ipa = download(assets[meta["id"] + ".ipa"], os.path.join(tmp, meta["id"] + ".ipa"))
        upload(app["id"], ipa, meta["version"], meta["build"])


if __name__ == "__main__":
    main()
