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

def ensure_internal_testing(email):
    """Per app: an internal TestFlight group that gets every build, with `email` in it."""
    users = get_all("/v1/users", {"fields[users]": "username,firstName,lastName,roles"})
    me = next((u for u in users if u["attributes"]["username"].lower() == email.lower()), None)
    if me is None:
        print("no App Store Connect user", email, "- users:", [u["attributes"]["username"] for u in users])
    first, last = (me["attributes"]["firstName"], me["attributes"]["lastName"]) if me else ("", "")
    apps = {a["attributes"]["bundleId"]: a for a in get_all("/v1/apps", {"fields[apps]": "name,bundleId"})}
    for a in apps_cfg():
        app = apps.get(BUNDLE_PREFIX + a["id"])
        if app is None:
            print(f"{a['id']}: no app record yet")
            continue
        groups = get_all(f"/v1/apps/{app['id']}/betaGroups")
        g = next((x for x in groups if x["attributes"].get("isInternalGroup")), None)
        if g is None:
            g = call("POST", "/v1/betaGroups", {"data": {"type": "betaGroups",
                "attributes": {"name": "Keyan", "isInternalGroup": True, "hasAccessToAllBuilds": True},
                "relationships": {"app": {"data": {"type": "apps", "id": app["id"]}}}}})["data"]
            print(f"{a['id']}: created internal group")
        testers = get_all(f"/v1/betaGroups/{g['id']}/betaTesters", {"fields[betaTesters]": "email"})
        if any(t["attributes"].get("email", "").lower() == email.lower() for t in testers):
            print(f"{a['id']}: {email} already testing")
            continue
        r = requests.post(API + "/v1/betaTesters", timeout=60, headers={"Authorization": "Bearer " + token()},
            json={"data": {"type": "betaTesters", "attributes": {"email": email, "firstName": first, "lastName": last},
                           "relationships": {"betaGroups": {"data": [{"type": "betaGroups", "id": g["id"]}]}}}})
        print(f"{a['id']}: add tester -> {r.status_code} {r.text[:300] if r.status_code >= 300 else ''}")

def builds_status():
    apps = {a["attributes"]["bundleId"]: a for a in get_all("/v1/apps", {"fields[apps]": "name,bundleId"})}
    for a in apps_cfg():
        app = apps.get(BUNDLE_PREFIX + a["id"])
        if app is None:
            print(f"{a['id']:7} no app record"); continue
        bs = call("GET", "/v1/builds", params={"filter[app]": app["id"], "sort": "-uploadedDate", "limit": 1,
                  "fields[builds]": "version,processingState,uploadedDate,expired", "include": "preReleaseVersion"})
        if not bs["data"]:
            print(f"{a['id']:7} {app['attributes']['name']}: no builds"); continue
        b = bs["data"][0]["attributes"]
        pv = (bs.get("included") or [{}])[0].get("attributes", {}).get("version", "?")
        print(f"{a['id']:7} {app['attributes']['name']}: {pv} ({b['version']}) {b['processingState']} {b['uploadedDate'][:16]}")

if __name__ == "__main__" and sys.argv[1] == "testers":
    ensure_internal_testing(sys.argv[2])
if __name__ == "__main__" and sys.argv[1] == "status":
    builds_status()

def rename_apps():
    """Set each App Store Connect record's name (and the bundle ID's label) from apps.json."""
    apps = {a["attributes"]["bundleId"]: a for a in get_all("/v1/apps", {"fields[apps]": "name,bundleId"})}
    bids = {b["attributes"]["identifier"]: b for b in get_all("/v1/bundleIds", {"fields[bundleIds]": "identifier,name"})}
    for a in apps_cfg():
        ident = BUNDLE_PREFIX + a["id"]
        b = bids.get(ident)
        if b and b["attributes"]["name"] != a["name"]:
            call("PATCH", f"/v1/bundleIds/{b['id']}", {"data": {"type": "bundleIds", "id": b["id"], "attributes": {"name": a["name"]}}})
        app = apps.get(ident)
        if app is None:
            print(f"{a['id']}: no app record"); continue
        want = a.get("store_name", a["name"])  # App Store names are global; a few needed a longer one
        if app["attributes"]["name"] == want:
            print(f"{a['id']}: already {want}"); continue
        infos = get_all(f"/v1/apps/{app['id']}/appInfos")
        info = next((i for i in infos if i["attributes"].get("appStoreState") in (None, "PREPARE_FOR_SUBMISSION")
                     or i["attributes"].get("state") in ("PREPARE_FOR_SUBMISSION",)), infos[0])
        locs = get_all(f"/v1/appInfos/{info['id']}/appInfoLocalizations")
        for loc in locs:
            r = requests.patch(API + f"/v1/appInfoLocalizations/{loc['id']}", timeout=60,
                               headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"},
                               json={"data": {"type": "appInfoLocalizations", "id": loc["id"], "attributes": {"name": want}}})
            msg = "ok" if r.status_code < 300 else f"{r.status_code} {r.text[:200]}"
            print(f"{a['id']}: {app['attributes']['name']} -> {want} [{loc['attributes'].get('locale')}] {msg}")

if __name__ == "__main__" and sys.argv[1] == "rename":
    rename_apps()
