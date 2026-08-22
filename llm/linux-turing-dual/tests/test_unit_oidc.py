"""Login: PKCE, and the token verification that decides who someone is.

Every test signs its own tokens with a LOCALLY GENERATED keypair published as a
fake JWKS. Nothing here touches the real identity provider -- a test that needs
a live pool is a test that gets skipped and then rots.

The rule these tests exist to defend (design section 2.1, and already encoded as
a test in the operator's own Go services): only `sub` is identity. `email` and
`cognito:username` are caller-mutable, and groups must come from the verified
token, never a header.
"""
import base64
import importlib.util
import json
import pathlib
import sys
import time

import jwt
import pytest

from nodescripts import load_script
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa

SCRIPTS = pathlib.Path(__file__).resolve().parent.parent / "scripts"

ISSUER = "https://issuer.example.org/pool"
AUDIENCE = "test-client-id"
KID = "test-key-1"




oidc = load_script("oidc")


@pytest.fixture(scope="module")
def keypair():
    return rsa.generate_private_key(public_exponent=65537, key_size=2048)


def _b64u(n: int) -> str:
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return base64.urlsafe_b64encode(raw).decode().rstrip("=")


@pytest.fixture(scope="module")
def jwks(keypair):
    pub = keypair.public_key().public_numbers()
    return {"keys": [{"kty": "RSA", "kid": KID, "alg": "RS256", "use": "sig",
                      "n": _b64u(pub.n), "e": _b64u(pub.e)}]}


def _sign(keypair, claims, kid=KID, alg="RS256"):
    pem = keypair.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )
    return jwt.encode(claims, pem, algorithm=alg, headers={"kid": kid})


def _claims(**over):
    now = int(time.time())
    c = {"iss": ISSUER, "aud": AUDIENCE, "sub": "subject-1", "token_use": "id",
         "exp": now + 600, "iat": now, "email": "person@example.org",
         "cognito:groups": ["llm-users"]}
    c.update(over)
    return c


# --- PKCE -------------------------------------------------------------------

def test_pkce_pair_is_a_valid_s256_challenge():
    verifier, challenge = oidc.pkce_pair()
    assert 43 <= len(verifier) <= 128
    import hashlib
    expect = base64.urlsafe_b64encode(
        hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")
    assert challenge == expect
    assert "=" not in challenge


def test_pkce_pairs_are_unique():
    assert len({oidc.pkce_pair()[0] for _ in range(200)}) == 200


def test_authorize_url_carries_everything_the_provider_needs():
    u = oidc.authorize_url("login.example.org", AUDIENCE,
                           "https://node.example.org/auth/callback",
                           "state-abc", "challenge-xyz")
    assert u.startswith("https://login.example.org/oauth2/authorize?")
    for frag in ("response_type=code", "client_id=" + AUDIENCE,
                 "code_challenge_method=S256", "code_challenge=challenge-xyz",
                 "state=state-abc", "scope=openid"):
        assert frag in u
    # A public client must never carry a secret in the URL.
    assert "client_secret" not in u


def test_token_form_is_urlencoded_and_carries_the_verifier():
    body = oidc.token_form(AUDIENCE, "the-code",
                           "https://node.example.org/auth/callback", "the-verifier")
    assert isinstance(body, bytes)
    s = body.decode()
    assert "grant_type=authorization_code" in s
    assert "code_verifier=the-verifier" in s
    assert "client_secret" not in s


# --- token verification -----------------------------------------------------

def test_a_valid_token_verifies_and_yields_sub(keypair, jwks):
    claims = oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                  issuer=ISSUER, audience=AUDIENCE,
                                  now=int(time.time()))
    assert claims["sub"] == "subject-1"


@pytest.mark.parametrize("over,why", [
    ({"exp": int(time.time()) - 10}, "expired"),
    ({"aud": "someone-elses-client"}, "wrong audience"),
    ({"iss": "https://evil.example.org/pool"}, "wrong issuer"),
    ({"token_use": "access"}, "wrong token_use"),
    ({"sub": ""}, "empty subject"),
])
def test_bad_tokens_are_refused(keypair, jwks, over, why):
    with pytest.raises(ValueError):
        oidc.verify_id_token(_sign(keypair, _claims(**over)), jwks,
                             issuer=ISSUER, audience=AUDIENCE,
                             now=int(time.time()))
    # Positive control: the same call path still ACCEPTS a good token, so
    # this test cannot pass because verification broke outright.
    assert oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                issuer=ISSUER, audience=AUDIENCE,
                                now=int(time.time()))["sub"] == "subject-1"



def test_an_unknown_kid_is_refused(keypair, jwks):
    with pytest.raises(ValueError):
        oidc.verify_id_token(_sign(keypair, _claims(), kid="not-in-the-jwks"),
                             jwks, issuer=ISSUER, audience=AUDIENCE,
                             now=int(time.time()))
    # Positive control: the same call path still ACCEPTS a good token, so
    # this test cannot pass because verification broke outright.
    assert oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                issuer=ISSUER, audience=AUDIENCE,
                                now=int(time.time()))["sub"] == "subject-1"



def test_a_token_signed_by_another_key_is_refused(keypair, jwks):
    other = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    with pytest.raises(ValueError):
        oidc.verify_id_token(_sign(other, _claims()), jwks,
                             issuer=ISSUER, audience=AUDIENCE,
                             now=int(time.time()))
    # Positive control: the same call path still ACCEPTS a good token, so
    # this test cannot pass because verification broke outright.
    assert oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                issuer=ISSUER, audience=AUDIENCE,
                                now=int(time.time()))["sub"] == "subject-1"



def test_alg_none_is_refused(keypair, jwks):
    """The classic JWT bypass: an unsigned token asserting whatever it likes."""
    header = base64.urlsafe_b64encode(
        json.dumps({"alg": "none", "kid": KID}).encode()).decode().rstrip("=")
    payload = base64.urlsafe_b64encode(
        json.dumps(_claims(sub="attacker")).encode()).decode().rstrip("=")
    with pytest.raises(ValueError):
        oidc.verify_id_token(f"{header}.{payload}.", jwks,
                             issuer=ISSUER, audience=AUDIENCE,
                             now=int(time.time()))
    # Positive control: the same call path still ACCEPTS a good token, so
    # this test cannot pass because verification broke outright.
    assert oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                issuer=ISSUER, audience=AUDIENCE,
                                now=int(time.time()))["sub"] == "subject-1"



def test_garbage_is_refused(keypair, jwks):
    for bad in ("", "not.a.token", "a.b", "....."):
        with pytest.raises(ValueError):
            oidc.verify_id_token(bad, jwks, issuer=ISSUER, audience=AUDIENCE,
                                 now=int(time.time()))
    # Positive control: the same call path still ACCEPTS a good token, so
    # this test cannot pass because verification broke outright.
    assert oidc.verify_id_token(_sign(keypair, _claims()), jwks,
                                issuer=ISSUER, audience=AUDIENCE,
                                now=int(time.time()))["sub"] == "subject-1"



# --- identity and groups ----------------------------------------------------

def test_groups_come_from_claims_only():
    assert oidc.groups(_claims(**{"cognito:groups": ["llm-admins", "Users"]})) \
        == ["llm-admins", "Users"]
    # No claim at all means no groups -- never a default.
    assert oidc.groups({"sub": "x"}) == []


def test_groups_ignores_a_non_list_claim():
    assert oidc.groups({"cognito:groups": "llm-admins"}) == []
    assert oidc.groups({"cognito:groups": None}) == []


def test_identity_takes_sub_and_treats_the_rest_as_presentation():
    sub, email, name = oidc.identity(_claims(email="a@example.org", name="A Person"))
    assert sub == "subject-1"
    assert email == "a@example.org" and name == "A Person"


def test_identity_refuses_a_missing_subject():
    with pytest.raises(ValueError):
        oidc.identity({"email": "a@example.org"})
    # Positive control, as above.
    assert oidc.identity(_claims())[0] == "subject-1"
