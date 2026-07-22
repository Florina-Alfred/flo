use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::rules::{Rule, Ruleset};

#[derive(Debug)]
pub enum RegistryError {
    Db(rusqlite::Error),
    Io(std::io::Error),
    BadName,
    Lock(String),
}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Db(e) => write!(f, "registry db error: {e}"),
            RegistryError::Io(e) => write!(f, "registry io error: {e}"),
            RegistryError::BadName => write!(f, "invalid ruleset_name"),
            RegistryError::Lock(e) => write!(f, "registry lock poisoned: {e}"),
        }
    }
}
impl std::error::Error for RegistryError {}

#[derive(Debug, PartialEq)]
pub enum RegisterOutcome {
    Inserted,
    Updated {
        version: u64,
        sha: String,
    },
    RejectedConflict,
    #[allow(dead_code)]
    Quarantined,
}

pub struct Registry {
    conn: Mutex<Connection>,
}

fn canonical_ruleset(rs: &Ruleset) -> String {
    let mut rs = rs.clone();
    rs.rules.sort_by(|a, b| a.name.cmp(&b.name));
    toml::to_string(&rs).expect("Ruleset serializable")
}

pub fn ruleset_digest(rs: &Ruleset) -> String {
    let mut h = Sha256::new();
    h.update(canonical_ruleset(rs).as_bytes());
    to_hex(&h.finalize())
}

#[allow(dead_code)]
pub fn rule_digest(rule: &Rule) -> String {
    let mut h = Sha256::new();
    h.update(toml::to_string(rule).expect("Rule serializable").as_bytes());
    to_hex(&h.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unix_timestamp_secs() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{s}")
}

impl Registry {
    pub fn new(path: &Path) -> Result<Self, RegistryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(RegistryError::Io)?;
        }
        let conn = Mutex::new(Connection::open(path).map_err(RegistryError::Db)?);
        conn.lock()
            .map_err(|e| RegistryError::Lock(e.to_string()))?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS audit (
                id INTEGER PRIMARY KEY,
                ts TEXT NOT NULL,
                name TEXT NOT NULL,
                owner TEXT NOT NULL,
                version INTEGER NOT NULL,
                sha TEXT NOT NULL,
                status TEXT NOT NULL,
                blob TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS registry (
                name TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                version INTEGER NOT NULL,
                sha TEXT NOT NULL
            );",
            )
            .map_err(RegistryError::Db)?;
        Ok(Self { conn })
    }

    pub fn publish(
        &self,
        rs: &Ruleset,
        _claiming_id: &str,
    ) -> Result<RegisterOutcome, RegistryError> {
        if !rs
            .ruleset_name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || rs.ruleset_name.is_empty()
            || rs.ruleset_name.len() > 64
        {
            return Err(RegistryError::BadName);
        }
        let sha = ruleset_digest(rs);
        let ts = unix_timestamp_secs();
        let existing: Option<(String, i64, String)> = self
            .conn
            .lock()
            .map_err(|e| RegistryError::Lock(e.to_string()))?
            .query_row(
                "SELECT owner, version, sha FROM registry WHERE name = ?",
                params![rs.ruleset_name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(RegistryError::Db)?;

        match existing {
            None => {
                self.conn
                    .lock()
                    .map_err(|e| RegistryError::Lock(e.to_string()))?
                    .execute(
                        "INSERT INTO registry (name, owner, version, sha) VALUES (?,?,?,?)",
                        params![rs.ruleset_name, rs.robot_owner, 1i64, sha],
                    )
                    .map_err(RegistryError::Db)?;
                self.write_audit(&ts, rs, 1, &sha, "inserted")?;
                Ok(RegisterOutcome::Inserted)
            }
            Some((owner, ver, prev_sha)) => {
                if owner != rs.robot_owner {
                    self.write_audit(&ts, rs, ver, &sha, "rejected_conflict")?;
                    return Ok(RegisterOutcome::RejectedConflict);
                }
                if prev_sha == sha {
                    return Ok(RegisterOutcome::Updated {
                        version: ver as u64,
                        sha,
                    });
                }
                let new_ver = ver + 1;
                self.conn
                    .lock()
                    .map_err(|e| RegistryError::Lock(e.to_string()))?
                    .execute(
                        "UPDATE registry SET owner=?, version=?, sha=? WHERE name=?",
                        params![rs.robot_owner, new_ver, sha, rs.ruleset_name],
                    )
                    .map_err(RegistryError::Db)?;
                self.write_audit(&ts, rs, new_ver, &sha, "updated")?;
                Ok(RegisterOutcome::Updated {
                    version: new_ver as u64,
                    sha,
                })
            }
        }
    }

    fn write_audit(
        &self,
        ts: &str,
        rs: &Ruleset,
        version: i64,
        sha: &str,
        status: &str,
    ) -> Result<(), RegistryError> {
        let blob = canonical_ruleset(rs);
        self.conn
            .lock()
            .map_err(|e| RegistryError::Lock(e.to_string()))?
            .execute(
                "INSERT INTO audit (ts, name, owner, version, sha, status, blob) VALUES (?,?,?,?,?,?,?)",
                params![ts, rs.ruleset_name, rs.robot_owner, version, sha, status, blob],
            )
            .map_err(RegistryError::Db)?;
        Ok(())
    }
}
