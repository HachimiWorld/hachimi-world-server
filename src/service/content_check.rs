//! Machine checks of user content before it is saved, judged by TypeSafe's Jev against a policy
//! kept in deployment config. See `docs/specs/content-check`.

use crate::config::Config;
use crate::db::content_check::{ContentCheck, ContentCheckDao, ContentPolicy, ContentPolicyDao};
use crate::util::typesafe::{SystemOneResponse, TypeSafeCfg, TypeSafeClient, TypeSafeStatusError};
use anyhow::{bail, Context};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;

const DECISION_FILE: &str = "decision.yaml";
const QUESTIONS_DIR: &str = "questions";

/// A username, checked when it changes.
/// @since 261016
pub const SUBJECT_USERNAME: &str = "user.username";
/// A profile bio, checked when it changes.
/// @since 261016
pub const SUBJECT_BIO: &str = "user.bio";
/// Every subject the server checks. A policy must cover exactly these.
const SUBJECTS: &[&str] = &[SUBJECT_USERNAME, SUBJECT_BIO];
/// Sent to TypeSafe at startup to try the policy; harmless under any policy.
const PROBE_TEXT: &str = "哈基米";

/// `content_check` in the config, read only when `content_check.enabled` is true; see [is_enabled].
/// @since 261016
#[derive(Debug, Clone, Deserialize)]
pub struct ContentCheckCfg {
    /// Holds `decision.yaml` and `questions/<subject>.json`. Kept out of the repository.
    pub policy_dir: PathBuf,
    /// The model for text, pinned to a version such as `jev-1.13.0`.
    pub model: String,
    pub typesafe: TypeSafeCfg,
}

/// Whether content checks are on. The `content_check` section is required and must say `enabled`,
/// so no instance runs without checks by accident; when it's false, nothing else in the section is
/// read, so the rest can stay in place for turning them back on.
/// @since 261017
pub fn is_enabled(config: &Config) -> anyhow::Result<bool> {
    if config.get("content_check")?.is_none() {
        bail!("Config [content_check] is required; set content_check.enabled to false to turn content checks off");
    }
    config.get_and_parse::<bool>("content_check.enabled")
}

/// @since 261016
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Pass,
    /// The questions that reached their threshold. For the record only; users aren't told.
    Blocked(Vec<String>),
}

/// A policy directory, loaded and checked for consistency.
/// @since 261016
#[derive(Debug, Clone)]
pub struct Policy {
    /// First 12 hex digits of the sha256 of the files.
    pub version: String,
    /// Relative path -> content.
    files: BTreeMap<String, String>,
    subjects: BTreeMap<String, SubjectPolicy>,
}

#[derive(Debug, Clone)]
struct SubjectPolicy {
    /// The API's `questions` map, sent as it is.
    questions: serde_json::Value,
    /// Question id -> the probability at or above which the content is blocked.
    block_at: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionFile {
    subjects: BTreeMap<String, DecisionSubject>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionSubject {
    block_at: BTreeMap<String, f64>,
}

impl Policy {
    /// Reads `decision.yaml` and `questions/*.json` under `dir`.
    pub fn load(dir: &Path) -> anyhow::Result<Policy> {
        let mut files = BTreeMap::new();
        let decision_path = dir.join(DECISION_FILE);
        let decision = std::fs::read_to_string(&decision_path)
            .with_context(|| format!("Failed to read {decision_path:?}"))?;
        files.insert(DECISION_FILE.to_string(), decision);

        let questions_dir = dir.join(QUESTIONS_DIR);
        for entry in std::fs::read_dir(&questions_dir).with_context(|| format!("Failed to read {questions_dir:?}"))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str())
                .with_context(|| format!("Unreadable file name: {path:?}"))?;
            let content = std::fs::read_to_string(&path).with_context(|| format!("Failed to read {path:?}"))?;
            files.insert(format!("{QUESTIONS_DIR}/{name}"), content);
        }
        Self::from_files(files)
    }

    /// Checks that the files agree: every subject in `decision.yaml` has a questions file and every
    /// questions file has a subject; every threshold names a noul question of its subject and lies
    /// in 0..=1. A mistake here would otherwise let content through without anyone noticing.
    fn from_files(files: BTreeMap<String, String>) -> anyhow::Result<Policy> {
        let decision: DecisionFile = yaml_serde::from_str(&files[DECISION_FILE])
            .with_context(|| format!("Invalid {DECISION_FILE}"))?;

        let mut subjects = BTreeMap::new();
        for (subject, decision) in decision.subjects {
            let path = format!("{QUESTIONS_DIR}/{subject}.json");
            let raw = files.get(&path).with_context(|| format!("{path} is missing for subject {subject}"))?;
            let questions: serde_json::Value = serde_json::from_str(raw).with_context(|| format!("Invalid JSON in {path}"))?;
            let map = questions.as_object().with_context(|| format!("{path} is not a JSON object"))?;
            if decision.block_at.is_empty() {
                bail!("Subject {subject} has no block_at, so it would never block anything");
            }
            for (id, threshold) in &decision.block_at {
                let question = map.get(id).with_context(|| format!("block_at of {subject} names {id}, which isn't in {path}"))?;
                if question.get("type").and_then(|t| t.as_str()) != Some("noul") {
                    bail!("block_at of {subject} names {id}, which isn't a noul question");
                }
                if !(0.0..=1.0).contains(threshold) {
                    bail!("block_at of {subject} for {id} is {threshold}, outside 0..=1");
                }
            }
            subjects.insert(subject, SubjectPolicy { questions, block_at: decision.block_at });
        }

        for path in files.keys() {
            if let Some(subject) = path.strip_prefix(&format!("{QUESTIONS_DIR}/")).and_then(|p| p.strip_suffix(".json"))
                && !subjects.contains_key(subject) {
                bail!("{path} has no subject in {DECISION_FILE}");
            }
        }

        Ok(Policy { version: version_of(&files), files, subjects })
    }

    /// Checks that the policy covers exactly `expected`: a missing subject would fail every check
    /// of it, and an extra one is most likely a typo.
    fn check_subjects(&self, expected: &[&str]) -> anyhow::Result<()> {
        for subject in expected {
            if !self.subjects.contains_key(*subject) {
                bail!("The policy has no subject {subject}");
            }
        }
        for subject in self.subjects.keys() {
            if !expected.contains(&subject.as_str()) {
                bail!("The policy has subject {subject}, which the server doesn't check");
            }
        }
        Ok(())
    }
}

/// Hashes each path and content in order, with separators so that moving bytes between a path and
/// its content changes the result.
fn version_of(files: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (path, content) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(content.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())[..12].to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The questions whose probability reached their threshold. Errors when an answer is missing,
/// which means the policy and the response don't match.
fn blocked_by(block_at: &BTreeMap<String, f64>, resp: &SystemOneResponse) -> Result<Vec<String>, String> {
    let mut blocked = Vec::new();
    for (id, threshold) in block_at {
        let p = resp.noul(id).ok_or_else(|| format!("The response has no noul answer for {id}"))?;
        if p >= *threshold {
            blocked.push(id.clone());
        }
    }
    Ok(blocked)
}

/// Checks content against the policy and records every check.
/// @since 261016
pub struct ContentChecker {
    policy: Policy,
    model: String,
    client: Arc<dyn TypeSafeClient>,
}

impl ContentChecker {
    /// Loads the policy and stores a snapshot of it if this version is new.
    /// Loads the policy, tries it against TypeSafe and stores a snapshot of it if this version is
    /// new. Fails on a mistake in the policy or the config; see [ContentChecker::probe].
    pub async fn new(cfg: &ContentCheckCfg, client: Arc<dyn TypeSafeClient>, pool: &PgPool) -> anyhow::Result<Self> {
        let policy = Policy::load(&cfg.policy_dir)?;
        policy.check_subjects(SUBJECTS)?;
        let checker = Self { policy, model: cfg.model.clone(), client };
        checker.probe().await?;
        store_snapshot(pool, &checker.policy).await?;
        Ok(checker)
    }

    /// Asks every subject's questions about a harmless text, which tries the key, the model and the
    /// questions at once. A rejection that retrying can't fix, or answers that don't match the
    /// policy, fail startup. A passing failure, such as a timeout or TypeSafe being down, only logs:
    /// an outage elsewhere shouldn't keep the server from starting, and checks pass meanwhile.
    async fn probe(&self) -> anyhow::Result<()> {
        for (subject, policy) in &self.policy.subjects {
            match self.client.system_one(&self.model, PROBE_TEXT, &policy.questions).await {
                Ok(resp) => {
                    if let Err(e) = blocked_by(&policy.block_at, &resp) {
                        bail!("TypeSafe's answers for {subject} don't match the policy: {e}");
                    }
                }
                Err(e) => match e.downcast_ref::<TypeSafeStatusError>() {
                    Some(status) if status.is_config_error() => {
                        return Err(e.context(format!("TypeSafe rejected the questions for {subject} with model {}; check the key, the model and the policy", self.model)));
                    }
                    _ => {
                        warn!("Couldn't try the content check policy against TypeSafe, starting anyway: {e:#}");
                        return Ok(());
                    }
                },
            }
        }
        Ok(())
    }

    pub fn policy_version(&self) -> &str {
        &self.policy.version
    }

    /// Checks `text` as `subject` and records it. When TypeSafe fails or its answer doesn't match
    /// the policy, the text passes: the error is recorded and the caller goes on.
    pub async fn check_text(
        &self,
        pool: &PgPool,
        subject: &str,
        actor_uid: i64,
        target_id: i64,
        text: &str,
    ) -> anyhow::Result<Verdict> {
        let policy = self.policy.subjects.get(subject)
            .with_context(|| format!("No content check policy for {subject}"))?;

        let started = Instant::now();
        let result = self.client.system_one(&self.model, text, &policy.questions).await;
        let latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;

        let (model, answers, error, blocked) = match result {
            Ok(resp) => match blocked_by(&policy.block_at, &resp) {
                Ok(blocked) => (Some(resp.model), Some(resp.answers), None, blocked),
                Err(e) => (Some(resp.model), Some(resp.answers), Some(e), Vec::new()),
            },
            Err(e) => (None, None, Some(format!("{e:#}")), Vec::new()),
        };
        if let Some(e) = &error {
            warn!(subject, actor_uid, "Content check failed, passing the content: {e}");
        }

        ContentCheckDao::insert(pool, &ContentCheck {
            id: 0,
            subject: subject.to_string(),
            target_id,
            actor_uid,
            content: text.to_string(),
            content_hash: sha256_hex(text.as_bytes()),
            policy_version: self.policy.version.clone(),
            model,
            answers,
            error,
            blocked_by: blocked.clone(),
            latency_ms,
            create_time: Utc::now(),
        }).await?;

        Ok(if blocked.is_empty() { Verdict::Pass } else { Verdict::Blocked(blocked) })
    }
}

async fn store_snapshot(pool: &PgPool, policy: &Policy) -> anyhow::Result<()> {
    if ContentPolicyDao::get_by_version(pool, &policy.version).await?.is_some() {
        return Ok(());
    }
    let snapshot = ContentPolicy {
        version: policy.version.clone(),
        files: serde_json::to_value(&policy.files)?,
        create_time: Utc::now(),
    };
    match ContentPolicyDao::insert(pool, &snapshot).await {
        Ok(()) => Ok(()),
        // Another instance starting at the same time stored it first
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUESTIONS: &str = r#"{
        "politics": { "type": "noul", "instructions": "…" },
        "sexual": { "type": "noul", "instructions": "…" },
        "tone": { "type": "choice", "instructions": "…", "criteria": { "a": null, "b": null } }
    }"#;

    fn files(decision: &str, questions: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut files = BTreeMap::from([(DECISION_FILE.to_string(), decision.to_string())]);
        for (subject, content) in questions {
            files.insert(format!("{QUESTIONS_DIR}/{subject}.json"), content.to_string());
        }
        files
    }

    fn error_of(decision: &str, questions: &[(&str, &str)]) -> String {
        format!("{:#}", Policy::from_files(files(decision, questions)).unwrap_err())
    }

    const DECISION: &str = "subjects:\n  user.bio:\n    block_at: { politics: 0.7, sexual: 0.7 }\n";

    #[test]
    fn loads_a_consistent_policy() {
        let policy = Policy::from_files(files(DECISION, &[("user.bio", QUESTIONS)])).unwrap();
        assert_eq!(policy.version.len(), 12);
        assert_eq!(policy.subjects["user.bio"].block_at["politics"], 0.7);
    }

    #[test]
    fn version_changes_with_any_file() {
        let a = Policy::from_files(files(DECISION, &[("user.bio", QUESTIONS)])).unwrap();
        let b = Policy::from_files(files(DECISION, &[("user.bio", &QUESTIONS.replace('…', "?"))])).unwrap();
        let c = Policy::from_files(files(&DECISION.replace("0.7, sexual", "0.8, sexual"), &[("user.bio", QUESTIONS)])).unwrap();
        assert_ne!(a.version, b.version);
        assert_ne!(a.version, c.version);
        assert_eq!(a.version, Policy::from_files(files(DECISION, &[("user.bio", QUESTIONS)])).unwrap().version);
    }

    #[test]
    fn rejects_inconsistent_policies() {
        assert!(error_of(DECISION, &[]).contains("questions/user.bio.json is missing"));
        assert!(error_of(DECISION, &[("user.bio", QUESTIONS), ("user.username", QUESTIONS)]).contains("questions/user.username.json has no subject"));
        assert!(error_of(&DECISION.replace("sexual", "spam"), &[("user.bio", QUESTIONS)]).contains("names spam, which isn't in"));
        assert!(error_of(&DECISION.replace("sexual", "tone"), &[("user.bio", QUESTIONS)]).contains("isn't a noul question"));
        assert!(error_of(&DECISION.replace("0.7, sexual", "1.5, sexual"), &[("user.bio", QUESTIONS)]).contains("outside 0..=1"));
        assert!(error_of("subjects:\n  user.bio:\n    block_at: {}\n", &[("user.bio", QUESTIONS)]).contains("never block"));
        assert!(error_of("subjects:\n  user.bio:\n    block_at: { politics: 0.7 }\n    reasons: {}\n", &[("user.bio", QUESTIONS)]).contains("Invalid decision.yaml"));
        assert!(error_of(DECISION, &[("user.bio", "[]")]).contains("is not a JSON object"));
    }

    #[test]
    fn is_enabled_must_be_said() {
        let enabled = |yaml: &str| is_enabled(&Config::parse_by_str(yaml).unwrap());
        assert!(enabled("server: {}").unwrap_err().to_string().contains("required"));
        assert!(!enabled("content_check:\n  enabled: false\n").unwrap());
        assert!(enabled("content_check:\n  enabled: true\n  model: jev-1.13.0\n").unwrap());
        assert!(enabled("content_check:\n  model: jev-1.13.0\n").is_err());
    }

    #[test]
    fn requires_exactly_the_subjects_the_server_checks() {
        let policy = Policy::from_files(files(DECISION, &[("user.bio", QUESTIONS)])).unwrap();
        assert!(policy.check_subjects(&["user.bio"]).is_ok());
        assert!(policy.check_subjects(&["user.bio", "user.username"]).unwrap_err().to_string().contains("no subject user.username"));
        assert!(policy.check_subjects(&[]).unwrap_err().to_string().contains("doesn't check"));
    }

    #[test]
    fn blocks_at_or_above_the_threshold() {
        let block_at = BTreeMap::from([("politics".to_string(), 0.7), ("sexual".to_string(), 0.7)]);
        let resp = |politics: f64, sexual: f64| -> SystemOneResponse {
            serde_json::from_value(serde_json::json!({
                "model": "jev-1.13.0",
                "answers": {
                    "politics": { "type": "noul", "noul": politics },
                    "sexual": { "type": "noul", "noul": sexual }
                },
                "usage": { "input_tokens": 1 }
            })).unwrap()
        };
        assert_eq!(blocked_by(&block_at, &resp(0.69, 0.1)), Ok(vec![]));
        assert_eq!(blocked_by(&block_at, &resp(0.7, 0.1)), Ok(vec!["politics".to_string()]));
        assert_eq!(blocked_by(&block_at, &resp(0.9, 0.95)), Ok(vec!["politics".to_string(), "sexual".to_string()]));

        let missing: SystemOneResponse = serde_json::from_value(serde_json::json!({
            "model": "jev-1.13.0",
            "answers": { "politics": { "type": "noul", "noul": 0.1 } },
            "usage": { "input_tokens": 1 }
        })).unwrap();
        assert!(blocked_by(&block_at, &missing).unwrap_err().contains("no noul answer for sexual"));
    }

    #[test]
    fn loads_from_a_directory() {
        let dir = std::env::temp_dir().join(format!("content-check-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(QUESTIONS_DIR)).unwrap();
        std::fs::write(dir.join(DECISION_FILE), DECISION).unwrap();
        std::fs::write(dir.join(QUESTIONS_DIR).join("user.bio.json"), QUESTIONS).unwrap();
        std::fs::write(dir.join(QUESTIONS_DIR).join(".DS_Store"), "ignored").unwrap();
        let policy = Policy::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(policy.files.len(), 2);
    }
}
