#!/usr/bin/env python3

import getpass
import json
import os
import tempfile
from pathlib import Path

from supabase import create_client


def obj_get(obj, key, default=None):
    if obj is None:
        return default
    if isinstance(obj, dict):
        return obj.get(key, default)
    return getattr(obj, key, default)


def auth_path():
    override = os.getenv("EDGESWARM_AUTH_FILE")
    if override:
        return Path(override)

    data_root = os.getenv("XDG_DATA_HOME")
    if data_root:
        root = Path(data_root)
    else:
        root = Path.home() / ".local" / "share"

    return (
        root
        / "edgeswarm"
        / "unified-node"
        / "auth_session.json"
    )


def save_session(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)

    fd, temp_name = tempfile.mkstemp(
        prefix=".auth_session.",
        suffix=".tmp",
        dir=str(path.parent),
    )

    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(data, handle, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())

        os.chmod(temp_name, 0o600)
        os.replace(temp_name, path)
        os.chmod(path, 0o600)

    except Exception:
        try:
            os.unlink(temp_name)
        except OSError:
            pass
        raise


def main():
    supabase_url = (
        os.getenv("SUPABASE_URL")
        or os.getenv("EDGESWARM_SUPABASE_URL")
        or ""
    ).strip()

    supabase_key = (
        os.getenv("SUPABASE_ANON_KEY")
        or os.getenv("EDGESWARM_SUPABASE_ANON_KEY")
        or ""
    ).strip()

    if not supabase_url or not supabase_key:
        raise SystemExit(
            "Missing SUPABASE_URL / SUPABASE_ANON_KEY."
        )

    email = input("EdgeSwarm email: ").strip().lower()
    password = getpass.getpass("Password: ")

    client = create_client(
        supabase_url,
        supabase_key,
    )

    print("Signing in...")
    auth_response = client.auth.sign_in_with_password(
        {
            "email": email,
            "password": password,
        }
    )

    user = obj_get(auth_response, "user")
    if not user:
        raise SystemExit(
            "Login failed: no authenticated user returned."
        )

    authenticated_email = str(
        obj_get(user, "email", "") or ""
    ).strip().lower()

    if not authenticated_email:
        raise SystemExit(
            "Login failed: authenticated email missing."
        )

    factors = client.auth.mfa.list_factors()
    totp_factors = obj_get(factors, "totp", []) or []

    verified = [
        factor
        for factor in totp_factors
        if str(
            obj_get(factor, "status", "")
        ).lower() == "verified"
    ]

    if not verified:
        client.auth.sign_out()
        raise SystemExit(
            "No verified TOTP factor. "
            "Configure 2FA before unified-node login."
        )

    factor_id = obj_get(verified[0], "id")
    if not factor_id:
        raise SystemExit(
            "Verified TOTP factor has no id."
        )

    code = getpass.getpass("6-digit authenticator code: ")

    challenge = client.auth.mfa.challenge(
        {"factor_id": factor_id}
    )

    challenge_id = obj_get(challenge, "id")
    if not challenge_id:
        raise SystemExit(
            "MFA challenge did not return an id."
        )

    verify_response = client.auth.mfa.verify(
        {
            "factor_id": factor_id,
            "challenge_id": challenge_id,
            "code": code,
        }
    )

    session = (
        obj_get(verify_response, "session")
        or client.auth.get_session()
    )

    access_token = str(
        obj_get(session, "access_token", "") or ""
    ).strip()

    refresh_token = str(
        obj_get(session, "refresh_token", "") or ""
    ).strip()

    expires_at = obj_get(session, "expires_at")

    if not access_token or not refresh_token:
        raise SystemExit(
            "MFA succeeded but session tokens are missing."
        )

    destination = auth_path()

    save_session(
        destination,
        {
            "authFileVersion":
                "edgeswarm_unified_auth_v1",
            "providerEmail":
                authenticated_email,
            "accessToken":
                access_token,
            "refreshToken":
                refresh_token,
            "expiresAt":
                expires_at,
            "mfaVerified":
                True,
        },
    )

    print(f"AUTH_FILE={destination}")
    print("AUTH_SESSION_CREATED=true")
    print("MFA_VERIFIED=true")
    print("TOKEN_VALUE_PRINTED=false")
    print("LEGACY_AUTH_FILE_CHANGED=false")
    print("LEGACY_WALLET_CHANGED=false")


if __name__ == "__main__":
    main()
