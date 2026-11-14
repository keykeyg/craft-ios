#!/usr/bin/env python3
"""Encrypt / decrypt the signing bundle (secrets.enc) with the CRAFT_KEY passphrase.

  vault.py seal <dir> <out.enc>     (needs CRAFT_KEY in env)
  vault.py open <in.enc> <dir>
AES-256-GCM, key from scrypt(n=2^17). The bundle holds the App Store Connect
API key and the distribution certificate; nothing in it is readable without
the passphrase, which lives only in the repo's CRAFT_KEY secret.
"""
import io, os, sys, tarfile
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt

MAGIC = b"KGV1"

def key(salt):
    return Scrypt(salt=salt, length=32, n=2**17, r=8, p=1).derive(os.environ["CRAFT_KEY"].strip().encode())

cmd, a, b = sys.argv[1:4]
if cmd == "seal":
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as t:
        for name in sorted(os.listdir(a)):
            t.add(os.path.join(a, name), arcname=name)
    salt, nonce = os.urandom(16), os.urandom(12)
    open(b, "wb").write(MAGIC + salt + nonce + AESGCM(key(salt)).encrypt(nonce, buf.getvalue(), MAGIC))
elif cmd == "open":
    raw = open(a, "rb").read()
    assert raw[:4] == MAGIC, "not a vault file"
    salt, nonce, ct = raw[4:20], raw[20:32], raw[32:]
    data = AESGCM(key(salt)).decrypt(nonce, ct, MAGIC)
    os.makedirs(b, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as t:
        t.extractall(b, filter="data")
    for name in os.listdir(b):
        os.chmod(os.path.join(b, name), 0o600)
