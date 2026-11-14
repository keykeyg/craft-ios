#!/usr/bin/env python3
"""Encrypt / decrypt the signing bundle (secrets.enc) with the CRAFT_KEY passphrase.

  vault.py seal <dir> <out.enc>     (needs CRAFT_KEY in env)
  vault.py open <in.enc> <dir>
AES-256-GCM, key from scrypt(n=2^17). The bundle holds the distribution
certificate (no App Store Connect key: uploads happen elsewhere); nothing in it is readable without
the passphrase, which lives only in the repo's CRAFT_KEY secret.
"""
import io, os, sys, tarfile
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt

MAGIC = b"KGV2"
LOG2N = 19  # scrypt cost: 512 MiB and a couple of seconds per guess

def key(salt, log2n=LOG2N):
    return Scrypt(salt=salt, length=32, n=2**log2n, r=8, p=1).derive(os.environ["CRAFT_KEY"].strip().encode())

cmd, a, b = sys.argv[1:4]
if cmd == "seal":
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as t:
        for name in sorted(os.listdir(a)):
            t.add(os.path.join(a, name), arcname=name)
    salt, nonce = os.urandom(16), os.urandom(12)
    open(b, "wb").write(MAGIC + bytes([LOG2N]) + salt + nonce + AESGCM(key(salt)).encrypt(nonce, buf.getvalue(), MAGIC))
elif cmd == "open":
    raw = open(a, "rb").read()
    assert raw[:4] == MAGIC, "not a vault file"
    log2n, salt, nonce, ct = raw[4], raw[5:21], raw[21:33], raw[33:]
    data = AESGCM(key(salt, log2n)).decrypt(nonce, ct, MAGIC)
    os.makedirs(b, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as t:
        t.extractall(b, filter="data")
    for name in os.listdir(b):
        os.chmod(os.path.join(b, name), 0o600)
