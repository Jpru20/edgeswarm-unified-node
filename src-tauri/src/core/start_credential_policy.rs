#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartCredentialActionV1 {
    RefreshPersistedCredential,
    UsePersistedCredential,
}

pub fn decide_start_credential_action_v1(
    authenticated: bool,
    in_memory_password_available: bool,
    persisted_credential_available: bool,
) -> Result<StartCredentialActionV1, String> {
    if !authenticated {
        return Err(
            "node_start_requires_authenticated_session"
                .into()
        );
    }

    if in_memory_password_available {
        return Ok(
            StartCredentialActionV1::
                RefreshPersistedCredential
        );
    }

    if persisted_credential_available {
        return Ok(
            StartCredentialActionV1::
                UsePersistedCredential
        );
    }

    Err(
        "node_start_restart_credential_unavailable"
            .into()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_authenticated_login_uses_memory_password_v1() {
        assert_eq!(
            decide_start_credential_action_v1(
                true,
                true,
                false,
            )
            .unwrap(),
            StartCredentialActionV1::
                RefreshPersistedCredential
        );
    }

    #[test]
    fn restored_windows_session_uses_dpapi_credential_v1() {
        assert_eq!(
            decide_start_credential_action_v1(
                true,
                false,
                true,
            )
            .unwrap(),
            StartCredentialActionV1::
                UsePersistedCredential
        );
    }

    #[test]
    fn restored_macos_session_uses_broker_credential_v1() {
        assert_eq!(
            decide_start_credential_action_v1(
                true,
                false,
                true,
            )
            .unwrap(),
            StartCredentialActionV1::
                UsePersistedCredential
        );
    }

    #[test]
    fn missing_persisted_credential_fails_closed_v1() {
        assert_eq!(
            decide_start_credential_action_v1(
                true,
                false,
                false,
            )
            .unwrap_err(),
            "node_start_restart_credential_unavailable"
        );
    }

    #[test]
    fn unauthenticated_session_remains_blocked_v1() {
        assert_eq!(
            decide_start_credential_action_v1(
                false,
                false,
                true,
            )
            .unwrap_err(),
            "node_start_requires_authenticated_session"
        );
    }

    #[test]
    fn restored_session_can_restart_repeatedly_v1() {
        for _ in 0..2 {
            assert_eq!(
                decide_start_credential_action_v1(
                    true,
                    false,
                    true,
                )
                .unwrap(),
                StartCredentialActionV1::
                    UsePersistedCredential
            );
        }
    }
}
