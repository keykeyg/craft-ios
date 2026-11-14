#!/usr/bin/env python3
"""Small App Store Connect API helper (JWT auth with an API key).

Env: ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH (path to AuthKey_XXXX.p8).
"""
import base64, json, os, sys, time
import jwt, requests

API = "https://api.appstoreconnect.apple.com"

def token():
    key = open(os.environ["ASC_KEY_PATH"]).read()
    now = int(time.time())
    return jwt.encode({"iss": os.environ["ASC_ISSUER_ID"], "iat": now, "exp": now + 1100, "aud": "appstoreconnect-v1"},
                      key, algorithm="ES256", headers={"kid": os.environ["ASC_KEY_ID"], "typ": "JWT"})

def call(method, path, body=None, params=None, ok=(200, 201, 204)):
    r = requests.request(method, API + path, params=params, json=body,
                         headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"}, timeout=60)
    if r.status_code not in ok:
        raise SystemExit(f"{method} {path} -> {r.status_code}: {r.text[:1500]}")
    return r.json() if r.text else {}

def get_all(path, params=None):
    out, url, params = [], path, dict(params or {}, limit=200)
    while url:
        j = call("GET", url, params=params)
        out += j.get("data", [])
        nxt = j.get("links", {}).get("next")
        url, params = (nxt.replace(API, ""), None) if nxt else (None, None)
    return out

if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "ping":
        apps = get_all("/v1/apps", {"fields[apps]": "name,bundleId"})
        print("apps:", [(a["attributes"]["name"], a["attributes"]["bundleId"]) for a in apps])
        certs = get_all("/v1/certificates", {"fields[certificates]": "name,certificateType,expirationDate,displayName"})
        print("certs:", [(c["id"], c["attributes"]["certificateType"], c["attributes"]["displayName"], c["attributes"]["expirationDate"][:10]) for c in certs])
        bids = get_all("/v1/bundleIds", {"fields[bundleIds]": "identifier,name,seedId,platform"})
        print("bundleIds:", [(b["attributes"]["identifier"], b["attributes"]["seedId"]) for b in bids])

BUNDLE_PREFIX = "com.keyangodarzi.kg."

def apps_cfg():
    return json.load(open(os.path.join(os.path.dirname(__file__), "..", "apps.json")))

def ensure_bundle_ids():
    have = {b["attributes"]["identifier"]: b for b in get_all("/v1/bundleIds", {"fields[bundleIds]": "identifier,name,seedId"})}
    out = {}
    for a in apps_cfg():
        ident = BUNDLE_PREFIX + a["id"]
        if ident not in have:
            j = call("POST", "/v1/bundleIds", {"data": {"type": "bundleIds", "attributes": {
                "identifier": ident, "name": a["name"], "platform": "IOS"}}})
            have[ident] = j["data"]
            print("registered", ident)
        out[a["id"]] = have[ident]
    return out

def ensure_profiles(cert_id, outdir):
    os.makedirs(outdir, exist_ok=True)
    bids = ensure_bundle_ids()
    profiles = get_all("/v1/profiles", {"filter[profileType]": "IOS_APP_STORE", "include": "bundleId,certificates"})
    for a in apps_cfg():
        bid = bids[a["id"]]
        name = f"kg {a['id']} appstore"
        prof = next((p for p in profiles if p["attributes"]["name"] == name and p["attributes"]["profileState"] == "ACTIVE"), None)
        if prof is None:
            prof = call("POST", "/v1/profiles", {"data": {"type": "profiles", "attributes": {"name": name, "profileType": "IOS_APP_STORE"},
                "relationships": {"bundleId": {"data": {"type": "bundleIds", "id": bid["id"]}},
                                  "certificates": {"data": [{"type": "certificates", "id": cert_id}]}}}})["data"]
            print("created profile", name)
        open(os.path.join(outdir, a["id"] + ".mobileprovision"), "wb").write(base64.b64decode(prof["attributes"]["profileContent"]))
    print("team:", next(iter(bids.values()))["attributes"]["seedId"])

if __name__ == "__main__" and sys.argv[1] == "bundle-ids":
    for k, v in ensure_bundle_ids().items():
        print(k, v["attributes"]["identifier"], v["attributes"]["seedId"])
if __name__ == "__main__" and sys.argv[1] == "profiles":
    ensure_profiles(sys.argv[2], sys.argv[3])
