#!/usr/bin/env python3
"""Test issuer for agent identity federation checks.
  agentid.py init ISSUER_URL          -> key (0600), jwks.json, openid-configuration
  agentid.py mint SUB AUD [k=v ...]   -> prints a 10-minute RS256 token
The private key never leaves this directory."""
import base64, json, os, sys, time, uuid
from pathlib import Path
import jwt
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa

HERE = Path(__file__).resolve().parent
KEY, META = HERE / "issuer-key.pem", HERE / "issuer.json"

def b64u(n: int) -> str:
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()

def init(issuer: str) -> None:
    if not KEY.exists():
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        fd = os.open(KEY, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    key = serialization.load_pem_private_key(KEY.read_bytes(), None)
    pub = key.public_key().public_numbers()
    kid = "agentrix-test-1"
    (HERE / "public").mkdir(exist_ok=True)
    (HERE / "public" / ".well-known").mkdir(exist_ok=True)
    jwks = {"keys": [{"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": b64u(pub.n), "e": b64u(pub.e)}]}
    (HERE / "public" / "jwks.json").write_text(json.dumps(jwks))
    disc = {"issuer": issuer, "jwks_uri": f"{issuer}/jwks.json", "response_types_supported": ["id_token"],
            "subject_types_supported": ["public"], "id_token_signing_alg_values_supported": ["RS256"],
            "claims_supported": ["iss", "sub", "aud", "exp", "iat", "jti"]}
    (HERE / "public" / ".well-known" / "openid-configuration").write_text(json.dumps(disc))
    META.write_text(json.dumps({"issuer": issuer, "kid": kid}))
    print("issuer", issuer, "kid", kid)

def mint(sub: str, aud: str, extra: dict) -> None:
    meta = json.loads(META.read_text())
    now = int(time.time())
    claims = {"iss": meta["issuer"], "sub": sub, "aud": aud, "iat": now, "nbf": now - 5, "exp": now + 600, "jti": str(uuid.uuid4()), **extra}
    print(jwt.encode(claims, KEY.read_bytes(), algorithm="RS256", headers={"kid": meta["kid"]}))

if __name__ == "__main__":
    if sys.argv[1] == "init":
        init(sys.argv[2].rstrip("/"))
    elif sys.argv[1] == "mint":
        mint(sys.argv[2], sys.argv[3], dict(a.split("=", 1) for a in sys.argv[4:]))
