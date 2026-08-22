#!/usr/bin/env python3
"""Authorization-code + PKCE login, and the token verification behind it.

No network calls: the JWKS document is passed in by the caller, which keeps this
module unit-testable against a locally generated keypair and keeps the fetch (and
its caching) a single concern in the gateway. Fetching JWKS per request would put
a network round trip on the login path and, worse, invite a per-request cache
miss storm during a key rotation.

THE RULE THIS MODULE ENFORCES (design section 2.1): only `sub` is identity.
`email` and `cognito:username` are caller-mutable and may never be used as an
identity key or for authorization. Groups are read from the VERIFIED token and
from nowhere else -- never a request header, query parameter or body field.
"""
from __future__ import annotations

import base64
import hashlib
import secrets
import time
import urllib.parse

import jwt
from jwt import PyJWKSet

# The only algorithm accepted. Taken from OUR list, never from the token's own
# header -- trusting the header is how `alg: none` and algorithm-confusion
# attacks work.
ALGORITHMS = ["RS256"]

SCOPES = "openid email profile"


def pkce_pair() -> tuple[str, str]:
    """(verifier, S256 challenge). The verifier stays server-side; only the
    challenge is sent to the provider."""
    verifier = base64.urlsafe_b64encode(secrets.token_bytes(48)).decode().rstrip("=")
    digest = hashlib.sha256(verifier.encode("ascii")).digest()
    challenge = base64.urlsafe_b64encode(digest).decode().rstrip("=")
    return verifier, challenge


def authorize_url(domain: str, client_id: str, redirect_uri: str,
                  state: str, challenge: str, scopes: str = SCOPES) -> str:
    """The provider's hosted-login URL. `domain` is a host, with no scheme."""
    q = urllib.parse.urlencode({
        "response_type": "code",
        "client_id": client_id,
        "redirect_uri": redirect_uri,
        "state": state,
        "scope": scopes,
        "code_challenge_method": "S256",
        "code_challenge": challenge,
    })
    return f"https://{domain}/oauth2/authorize?{q}"


def token_form(client_id: str, code: str, redirect_uri: str,
               verifier: str) -> bytes:
    """Body for the token exchange. No client secret: this is a PUBLIC client,
    and PKCE is what replaces the secret."""
    return urllib.parse.urlencode({
        "grant_type": "authorization_code",
        "client_id": client_id,
        "code": code,
        "redirect_uri": redirect_uri,
        "code_verifier": verifier,
    }).encode("ascii")


def token_url(domain: str) -> str:
    return f"https://{domain}/oauth2/token"


def jwks_url(region: str, pool_id: str) -> str:
    return (f"https://cognito-idp.{region}.amazonaws.com/{pool_id}"
            f"/.well-known/jwks.json")


def issuer_url(region: str, pool_id: str) -> str:
    return f"https://cognito-idp.{region}.amazonaws.com/{pool_id}"


def _signing_key(token: str, jwks: dict):
    """Resolve the token's `kid` against the JWKS, or raise ValueError.

    Split out of verify_id_token so that function stays readable: key resolution
    is four distinct failure modes (malformed JWKS, unreadable header, missing
    kid, unknown kid) and none of them is about the claims.
    """
    try:
        key_set = PyJWKSet.from_dict(jwks)
    except Exception as exc:                      # malformed or empty JWKS
        raise ValueError(f"unusable JWKS: {exc}") from exc

    try:
        kid = jwt.get_unverified_header(token).get("kid")
    except Exception as exc:
        raise ValueError(f"unreadable header: {exc}") from exc
    if not kid:
        raise ValueError("token carries no kid")

    for candidate in key_set.keys:
        if candidate.key_id == kid:
            return candidate
    # Callers may refresh the JWKS once and retry: key rotation is routine.
    raise ValueError("kid is not in the JWKS")


def verify_id_token(token: str, jwks: dict, *, issuer: str, audience: str,
                    now: int | None = None) -> dict:
    """Verify signature, issuer, audience, expiry and token_use.

    Raises ValueError on ANY failure. The message is safe to log but must not be
    returned to a caller: telling an unauthenticated client "expired" rather than
    "wrong audience" is an oracle.
    """
    if not token or not isinstance(token, str) or token.count(".") != 2:
        raise ValueError("token is not a JWS")

    signing = _signing_key(token, jwks)

    # Signature, audience and issuer are PyJWT's job. Expiry is checked here
    # instead, against an INJECTABLE clock -- PyJWT has no clock parameter, and a
    # test that cannot control time either sleeps or asserts nothing.
    options = {"require": ["exp", "iat", "sub", "aud", "iss"],
               "verify_exp": False}
    try:
        claims = jwt.decode(token, signing.key, algorithms=ALGORITHMS,
                            audience=audience, issuer=issuer, options=options,
                            leeway=0)
    except Exception as exc:
        raise ValueError(f"token rejected: {exc}") from exc

    at = int(time.time()) if now is None else int(now)
    exp = claims.get("exp")
    if not isinstance(exp, (int, float)) or at >= exp:
        raise ValueError("token has expired")

    # `token_use` is provider-specific and worth asserting where present: an
    # ACCESS token verifies against the same JWKS and carries the same issuer,
    # so without this it would be accepted as an identity assertion. A provider
    # that does not emit the claim at all is still accepted.
    if claims.get("token_use") not in ("id", None):
        raise ValueError("token_use is not 'id'")
    if not claims.get("sub"):
        raise ValueError("token carries no subject")
    return claims


def groups(claims: dict) -> list[str]:
    """Group names from the verified claims. Absent, null or non-list means NO
    groups -- never a default, and never read from anywhere but the token."""
    g = (claims or {}).get("cognito:groups")
    if isinstance(g, list):
        return [str(x) for x in g]
    return []


def identity(claims: dict) -> tuple[str, str | None, str | None]:
    """(sub, email, display name). `sub` is identity; the other two are
    presentation only, refreshed from the verified token on each login and never
    used for authorization."""
    sub = (claims or {}).get("sub")
    if not sub:
        raise ValueError("claims carry no subject")
    name = claims.get("name")
    if not name:
        parts = [claims.get("given_name"), claims.get("family_name")]
        name = " ".join(p for p in parts if p) or None
    return str(sub), claims.get("email"), name
