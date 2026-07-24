use sha2::{Digest, Sha256};

use crate::rules::Ruleset;

pub struct MutatedRuleset {
    pub ruleset: Ruleset,
    pub sha: String,
}

pub fn compute_sha(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn mutate_ruleset(ruleset: Ruleset) -> MutatedRuleset {
    let raw_toml = toml::to_string(&ruleset).expect("Ruleset is serializable");
    let sha = compute_sha(raw_toml.as_bytes());
    MutatedRuleset { ruleset, sha }
}

pub fn is_same_as_last(raw_toml: &str, last_sha: &str) -> bool {
    let sha = compute_sha(raw_toml.as_bytes());
    sha == last_sha
}
