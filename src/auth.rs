//! Client/server authentication model for the `flo-engine`.
//!
//! Per the flo-engine PRD (§2), authentication is an explicit axis:
//! `none | mtls | ed25519`. The default is `mtls`. `none` is permitted only for
//! loopback/air-gapped development; production must hard-block it unless an
//! explicit `allow_insecure` override is set. The server extracts a canonical
//! `robot_id` from the client certificate SAN; `none` falls back to the
//! `robot_owner`/launch flag (no ACL enforcement).
//!
//! This module is transport-agnostic config: it builds the `zenoh::Config` for
//! each mode. Standing up the actual router process is a separate concern.

use std::path::PathBuf;

use zenoh::Config;

/// Authentication mode for a `flo` node (client or server).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// mTLS: Zenoh `enable_mtls` with a fleet CA / server allowlist. Default.
    #[default]
    Mtls,
    /// ed25519 keypair + server allowlist (opt-in; not yet wired to a handshake).
    Ed25519,
    /// No authentication. Dev/air-gapped only; rejected in production unless
    /// `allow_insecure` is set.
    None,
}

impl AuthMode {
    /// Parse from a config string; case-insensitive. Unknown values error.
    pub fn parse(s: &str) -> Result<Self, AuthError> {
        match s.to_ascii_lowercase().as_str() {
            "mtls" => Ok(AuthMode::Mtls),
            "ed25519" => Ok(AuthMode::Ed25519),
            "none" => Ok(AuthMode::None),
            other => Err(AuthError::UnknownMode(other.to_string())),
        }
    }

    /// Whether this mode performs any authentication.
    pub fn is_authenticated(&self) -> bool {
        !matches!(self, AuthMode::None)
    }
}

/// Authentication configuration for a `flo` node.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthConfig {
    /// Selected mode (default `mtls`).
    pub mode: AuthMode,
    /// Explicitly permit `none` in a production deployment. Off by default;
    /// setting it prints a loud warning and disables impersonation protection.
    pub allow_insecure: bool,
    /// Path to the node's certificate (PEM). mTLS client/server only.
    pub cert: Option<PathBuf>,
    /// Path to the node's private key (PEM). mTLS client/server only.
    pub key: Option<PathBuf>,
    /// Trust anchor: CA cert (mTLS) or allowlist of authorized public keys
    /// (ed25519). mTLS/ed25519 only.
    pub trust: Option<PathBuf>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::Mtls,
            allow_insecure: false,
            cert: None,
            key: None,
            trust: None,
        }
    }
}

/// Errors from auth configuration.
#[derive(Debug, PartialEq)]
pub enum AuthError {
    /// Unknown auth mode string.
    UnknownMode(String),
    /// `auth: none` requested in a production context without `allow_insecure`.
    InsecureBlocked,
    /// A required credential file (cert/key/trust) is missing for the mode.
    MissingCredential(&'static str),
}

// Minimal Display impl (avoids pulling a derive dep); keeps `AuthError` usable
// in `anyhow`/`Box<dyn Error>` chains.
impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::UnknownMode(m) => write!(f, "unknown auth mode: {m}"),
            AuthError::InsecureBlocked => write!(
                f,
                "auth: none is blocked in production; set allow_insecure to override (dev/air-gapped only)"
            ),
            AuthError::MissingCredential(c) => write!(f, "missing required credential: {c}"),
        }
    }
}

impl std::error::Error for AuthError {}

impl AuthConfig {
    /// Validate the config for a production deployment. `auth: none` is rejected
    /// unless `allow_insecure` is set. Authenticated modes require their
    /// credential files to be present. Returns the validated config (clone).
    pub fn validate_production(&self) -> Result<&Self, AuthError> {
        if self.mode == AuthMode::None && !self.allow_insecure {
            return Err(AuthError::InsecureBlocked);
        }
        self.check_credentials()?;
        Ok(self)
    }

    /// Validate for a dev/loopback deployment. `none` is always allowed here;
    /// authenticated modes still need their credential files.
    pub fn validate_dev(&self) -> Result<&Self, AuthError> {
        self.check_credentials()
    }

    fn check_credentials(&self) -> Result<&Self, AuthError> {
        match self.mode {
            AuthMode::None => Ok(self),
            AuthMode::Mtls | AuthMode::Ed25519 => {
                if self.cert.is_none() {
                    return Err(AuthError::MissingCredential("cert"));
                }
                if self.key.is_none() {
                    return Err(AuthError::MissingCredential("key"));
                }
                if self.trust.is_none() {
                    return Err(AuthError::MissingCredential("trust"));
                }
                Ok(self)
            }
        }
    }

    /// Build the Zenoh session config for this node. For `none`, returns the
    /// default peer config (the caller is responsible for the loopback/air-gap
    /// boundary). For `mtls`, enables Zenoh mTLS with the node cert/key and the
    /// trust anchor. ed25519 is recorded but not yet wired to a handshake
    /// (returns an error until implemented, so we fail closed).
    pub fn zenoh_config(&self) -> Result<Config, AuthError> {
        match self.mode {
            AuthMode::None => Ok(Config::default()),
            AuthMode::Mtls => {
                let mut c = Config::default();
                let cert = self
                    .cert
                    .as_ref()
                    .ok_or(AuthError::MissingCredential("cert"))?;
                let key = self
                    .key
                    .as_ref()
                    .ok_or(AuthError::MissingCredential("key"))?;
                let trust = self
                    .trust
                    .as_ref()
                    .ok_or(AuthError::MissingCredential("trust"))?;
                // Zenoh 1.x mTLS: enable_mtls + PEM cert/key + CA trust.
                let _ = c.insert_json5("transport/auth/usrpwd", "false");
                let _ = c.insert_json5("transport/auth/usrpwd", "false");
                let _ = c.insert_json5("transport/link/tls/enable_mtls", "true");
                let _ = c.insert_json5(
                    "transport/link/tls/certificate",
                    &format!("\"{}\"", cert.display()),
                );
                let _ = c.insert_json5("transport/link/tls/key", &format!("\"{}\"", key.display()));
                let _ = c.insert_json5(
                    "transport/link/tls/trust_anchors",
                    &format!("\"{}\"", trust.display()),
                );
                Ok(c)
            }
            AuthMode::Ed25519 => {
                // Recorded as a first-class opt-in; the handshake is a future
                // slice. Fail closed so we never silently run unauthenticated.
                Err(AuthError::MissingCredential(
                    "ed25519 handshake (not yet implemented)",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_mtls() {
        assert_eq!(AuthConfig::default().mode, AuthMode::Mtls);
    }

    #[test]
    fn parse_is_case_insensitive() {
        assert_eq!(AuthMode::parse("MTLS").unwrap(), AuthMode::Mtls);
        assert_eq!(AuthMode::parse("None").unwrap(), AuthMode::None);
        assert_eq!(AuthMode::parse("ed25519").unwrap(), AuthMode::Ed25519);
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(AuthMode::parse("kerberos").is_err());
    }

    #[test]
    fn production_blocks_none_without_override() {
        let cfg = AuthConfig {
            mode: AuthMode::None,
            allow_insecure: false,
            ..Default::default()
        };
        assert_eq!(cfg.validate_production(), Err(AuthError::InsecureBlocked));
    }

    #[test]
    fn production_allows_none_with_override() {
        let cfg = AuthConfig {
            mode: AuthMode::None,
            allow_insecure: true,
            ..Default::default()
        };
        assert!(cfg.validate_production().is_ok());
    }

    #[test]
    fn mtls_requires_credentials_in_production() {
        let cfg = AuthConfig {
            mode: AuthMode::Mtls,
            ..Default::default()
        };
        assert_eq!(
            cfg.validate_production(),
            Err(AuthError::MissingCredential("cert"))
        );
    }

    #[test]
    fn mtls_validates_with_all_credentials() {
        let cfg = AuthConfig {
            mode: AuthMode::Mtls,
            cert: Some(PathBuf::from("/etc/flo/cert.pem")),
            key: Some(PathBuf::from("/etc/flo/key.pem")),
            trust: Some(PathBuf::from("/etc/flo/ca.pem")),
            ..Default::default()
        };
        assert!(cfg.validate_production().is_ok());
    }

    #[test]
    fn dev_allows_none_without_override() {
        let cfg = AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        };
        assert!(cfg.validate_dev().is_ok());
    }

    #[test]
    fn ed25519_not_yet_wired_fails_closed() {
        let cfg = AuthConfig {
            mode: AuthMode::Ed25519,
            cert: Some(PathBuf::from("/etc/flo/cert.pem")),
            key: Some(PathBuf::from("/etc/flo/key.pem")),
            trust: Some(PathBuf::from("/etc/flo/allowlist.json")),
            ..Default::default()
        };
        assert!(cfg.zenoh_config().is_err());
    }

    #[test]
    fn none_builds_default_config() {
        let cfg = AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        };
        assert!(cfg.zenoh_config().is_ok());
    }
}
