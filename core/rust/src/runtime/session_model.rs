use std::fmt;

/// Stable identity for one process-local Hara session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        {
            return Err("INVALID_SESSION_NAME".into());
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Identity of one logical filesystem resource managed by the local Kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionMountId(u64);

impl SessionMountId {
    pub const fn new(value: u64) -> Self {
        assert!(value > 0, "filesystem mount identifiers must be positive");
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionMountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Observable lifecycle of one Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    New,
    Active,
    Closed,
}

impl SessionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Active => "active",
            Self::Closed => "closed",
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Authority inherited from an embedding host by one in-process evaluator.
///
/// This policy describes direct host authority only. A filesystem mounted
/// explicitly on a Session is a separate scoped delegation and does not turn
/// that Session into a host-filesystem session. In-process namespace and
/// Runtime separation remain logical isolation, not a security boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionAuthorityPolicy {
    pub host_filesystem: bool,
    pub host_network: bool,
    pub host_process: bool,
    pub reflection: bool,
    pub packages: bool,
    pub project: bool,
}

impl SessionAuthorityPolicy {
    pub const ZERO: Self = Self {
        host_filesystem: false,
        host_network: false,
        host_process: false,
        reflection: false,
        packages: false,
        project: false,
    };

    pub const fn profile(self) -> &'static str {
        if !self.host_filesystem
            && !self.host_network
            && !self.host_process
            && !self.reflection
            && !self.packages
            && !self.project
        {
            "zero"
        } else {
            "explicit"
        }
    }
}

/// Immutable construction contract for one ordinary in-process Session.
///
/// The Session owns the Runtime created from this specification. Live mounts
/// and provider implementations remain Kernel-owned resources and are attached
/// separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSpec {
    pub id: SessionId,
    pub authority: SessionAuthorityPolicy,
}

impl SessionSpec {
    pub fn new(id: SessionId, authority: SessionAuthorityPolicy) -> Self {
        Self { id, authority }
    }

    pub fn zero_authority(name: &str) -> Result<Self, String> {
        Ok(Self::new(
            SessionId::parse(name)?,
            SessionAuthorityPolicy::ZERO,
        ))
    }
}

/// Immutable status projection for one Session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStatus {
    pub name: SessionId,
    pub namespace: String,
    pub state: SessionState,
    pub filesystem: Option<SessionMountId>,
    pub authority: SessionAuthorityPolicy,
}

/// Compatibility name retained while the Session/Runtime model is tightened.
pub type SessionMetadata = SessionStatus;

#[cfg(test)]
mod session_model_tests {
    use super::*;

    #[test]
    fn session_identity_and_mount_identifiers_are_distinct_types() {
        let session = SessionId::parse("workspace.alpha").unwrap();
        let mount = SessionMountId::new(7);
        assert_eq!(session.as_str(), "workspace.alpha");
        assert_eq!(mount.get(), 7);
        assert_eq!(mount.to_string(), "7");
        assert!(SessionId::parse("bad/name").is_err());
    }

    #[test]
    #[should_panic(expected = "filesystem mount identifiers must be positive")]
    fn mount_identifiers_reject_zero() {
        SessionMountId::new(0);
    }

    #[test]
    fn zero_authority_spec_is_explicit_and_immutable() {
        let spec = SessionSpec::zero_authority("child").unwrap();
        assert_eq!(spec.id.as_str(), "child");
        assert_eq!(spec.authority, SessionAuthorityPolicy::ZERO);
        assert_eq!(spec.authority.profile(), "zero");
    }

    #[test]
    fn session_lifecycle_states_are_explicit() {
        assert_eq!(SessionState::New.as_str(), "new");
        assert_eq!(SessionState::Active.as_str(), "active");
        assert_eq!(SessionState::Closed.as_str(), "closed");
    }
}
