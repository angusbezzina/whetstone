//! The Virgil backend: admitted lessons in the group's Records `lesson`
//! collection and raised hands as tasks in the group's channel, reached only
//! through the `virgil` CLI on the daemon socket. No MCP tool, no door, no
//! database.
//!
//! `virgil lessons` (Virgil PRs #1021, #1036) has `remember`, `supersede`,
//! `forget`, `find`, `get`, `list` and `import-beads`, and no admit: a lesson
//! is current the moment it is written and its trust comes from the writer (a
//! person's write is validated, an agent's, or one carrying
//! `VIRGIL_VIA=agent:<client>`, inferred). So proposals stay in Whetstone's
//! own store until a person admits them, and the admit is that person's
//! `virgil lessons remember`. There is no `--group`: the scope is the
//! repository attachment found from the checkout (`.virgil/repo.toml`).
//!
//! Every value is passed as `--flag=value`, so text starting with `-` is never
//! read as a flag. Answers are one JSON line, `{"api_version","result"}` or
//! `{"api_version","error":{"code","message","hint"}}`. Virgil's exit codes
//! (0 ok, 1 internal, 2 invalid, 3 read again, 4 refused content, 5 not
//! permitted, 6 not found or unattached, 7 unavailable) are mapped to
//! Whetstone's refusal codes, never passed through. A write whose result
//! carries a record id landed, even at exit 7 (replication still arriving) or
//! 3 to 5 (a follow-up refused).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    ActorKind, Anchor, AnchorComparison, Audience, ContentDigest, ForgetProposal, Lesson,
    LessonDecision, LessonStamp, LessonStatus, LessonVerb, Subject, Trust, LESSON_SCHEMA_V1,
};
use crate::storage::ProjectLayout;

use super::store::{BeadsLessons, LessonStore, StoredLesson};
use super::Refusal;

pub const LESSONS_API: &str = "virgil.lessons/v1";
pub const TASKS_API: &str = "virgil.tasks/v1";

/// Virgil's limit on a lesson's subjects and anchors together.
pub const MAX_SUBJECTS: usize = 16;
const PAGE: u32 = 100;
const MAX_PAGES: usize = 100;

/// The program to run: `virgil` on PATH.
fn program() -> PathBuf {
    PathBuf::from("virgil")
}

/// One answer: the last JSON line, the exit code and stderr's last line.
struct Answer {
    line: Value,
    exit: i32,
    stderr: String,
}

/// Run `virgil` once. `via` is set on the child only, never in Whetstone's
/// own environment, so a person's call that follows an agent's carries none.
fn invoke(
    root: &Path,
    args: &[String],
    stdin: Option<&str>,
    via: Option<&str>,
) -> Result<Answer, Refusal> {
    let mut command = Command::new(program());
    command
        .args(args)
        .current_dir(root)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match via {
        Some(via) => command.env("VIRGIL_VIA", via),
        None => command.env_remove("VIRGIL_VIA"),
    };
    let mut child = command.spawn().map_err(|error| {
        Refusal::new(
            "store_unavailable",
            format!("virgil is not available ({error}); the memory switch names Virgil, so nothing was written. Install or start Virgil, or switch back with wh init --action memory --store beads."),
        )
    })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    let output = child.wait_with_output().map_err(|error| {
        Refusal::new(
            "store_unavailable",
            format!("virgil did not answer: {error}"),
        )
    })?;
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .unwrap_or(Value::Null);
    Ok(Answer {
        line,
        exit: output.status.code().unwrap_or(1),
        stderr: String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect(),
    })
}

/// A failed answer as Whetstone's refusal: 6 is told apart by `error.code`
/// (`unattached` is the checkout, `not_found` the lesson).
fn failure(what: &str, answer: &Answer) -> Refusal {
    let error = &answer.line["error"];
    let message = error["message"]
        .as_str()
        .filter(|message| !message.trim().is_empty())
        .map_or_else(
            || {
                if answer.stderr.is_empty() {
                    "no detail".to_string()
                } else {
                    answer.stderr.clone()
                }
            },
            str::to_owned,
        );
    let hint = error["hint"]
        .as_str()
        .filter(|hint| !hint.trim().is_empty())
        .map(|hint| format!(" ({hint})"))
        .unwrap_or_default();
    let detached = matches!(
        error["code"].as_str(),
        Some("unattached" | "repository_missing")
    );
    match answer.exit {
        2 => Refusal::new("invalid_request", format!("virgil {what}: {message}{hint}")),
        3 => Refusal::new(
            "read_again",
            format!("virgil {what}: the lesson moved since it was read ({message}). Read it again (wh dash) and repeat; nothing was retried."),
        ),
        4 => Refusal::new("refused_content", format!("virgil {what}: {message}{hint}")),
        5 => Refusal::new("not_permitted", format!("virgil {what}: {message}{hint}")),
        6 if detached => Refusal::new(
            "unattached",
            format!("This checkout is not attached to a Virgil repository ({message}). Attach the repository: an admin registers it (virgil repos register), a developer binds it to the channel (virgil repos bind), then virgil repos attach <repository> --group <group> writes .virgil/repo.toml; virgil repos show checks it. Nothing was written."),
        ),
        6 => Refusal::new("lesson_not_found", format!("virgil {what}: {message}")),
        7 => Refusal::new(
            "store_unavailable",
            format!("virgil {what}: {message}{hint}"),
        ),
        _ => Refusal::new("store_refused", format!("virgil {what} failed: {message}{hint}")),
    }
}

/// A deterministic idempotency key for Virgil (a ULID): the time part from
/// `at_ms`, the rest from Whetstone's own key, so a retry of the same request
/// sends the same key. A new lesson's record id is this key.
pub fn idempotency_ulid(key: &str, at_ms: u64) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let digest = Sha256::digest(key.as_bytes());
    let mut random = [0_u8; 16];
    random[6..].copy_from_slice(&digest[..10]);
    let mut value = u128::from_be_bytes(random) | (u128::from(at_ms & 0xFFFF_FFFF_FFFF) << 80);
    let mut text = [b'0'; 26];
    for slot in text.iter_mut().rev() {
        *slot = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8_lossy(&text).into_owned()
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Milliseconds since the epoch of a `YYYY-MM-DDTHH:MM:SSZ` timestamp (0 if
/// it is not one).
pub fn millis(at: &str) -> u64 {
    let number = |range: std::ops::Range<usize>| at.get(range)?.parse::<i64>().ok();
    let parts = (
        number(0..4),
        number(5..7),
        number(8..10),
        number(11..13),
        number(14..16),
        number(17..19),
    );
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = parts else {
        return 0;
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds * 1_000).unwrap_or(0)
}

/// The time a ULID was minted, as a `YYYY-MM-DDTHH:MM:SSZ` timestamp.
fn ulid_time(id: &str) -> Option<String> {
    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    if id.len() != 26 {
        return None;
    }
    let mut ms: i64 = 0;
    for character in id[..10].chars() {
        let digit = i64::try_from(ALPHABET.find(character.to_ascii_uppercase())?).ok()?;
        ms = ms.checked_mul(32)?.checked_add(digit)?;
    }
    let seconds = ms / 1_000;
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let time = seconds.rem_euclid(86_400);
    (1970..=9999).contains(&year).then(|| {
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            time / 3_600,
            time % 3_600 / 60,
            time % 60
        )
    })
}

/// What a write answered: the record and the trust Virgil derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub id: String,
    pub revision: u64,
    pub trust: Option<Trust>,
    pub via: String,
    /// Accepted while replication is still arriving (exit 7 with an id).
    pub pending: bool,
    /// Accepted, with a follow-up refused (exit 3 to 5 with an id).
    pub follow_up: Option<String>,
}

impl Written {
    /// The decision receipt Whetstone records: the Virgil record revision.
    pub fn receipt(&self) -> String {
        format!("virgil:{}@{}", self.id, self.revision)
    }
}

fn trust_of(value: &str) -> Option<Trust> {
    match value {
        "inferred" => Some(Trust::Inferred),
        "validated" => Some(Trust::Validated),
        "authoritative" => Some(Trust::Authoritative),
        _ => None,
    }
}

/// A `virgil lessons remember`: a new lesson, or with `if_revision` a rewrite
/// in place (which replaces `about` whole, so pass every subject and anchor).
#[derive(Debug, Clone)]
pub struct Remember<'a> {
    pub key: &'a str,
    pub text: &'a str,
    /// `channel` (the repository's bound channel) or `personal`.
    pub audience: &'a str,
    pub about: Vec<String>,
    pub anchors: Vec<String>,
    pub if_revision: Option<u64>,
    pub reason: Option<&'a str>,
    pub idempotency: String,
}

/// The `virgil lessons` commands, run from the checkout root, as `actor`.
pub struct VirgilCli {
    root: PathBuf,
    actor: ActorKind,
}

impl VirgilCli {
    pub fn new(project_root: &Path, actor: ActorKind) -> Self {
        Self {
            root: project_root.to_path_buf(),
            actor,
        }
    }

    /// `VIRGIL_VIA` for this actor's writes: `agent:<host>` for an agent,
    /// none for a person.
    pub fn via(&self) -> Option<String> {
        (self.actor == ActorKind::Agent)
            .then(|| format!("agent:{}", super::virgil_client(&super::agent_host())))
    }

    fn read(&self, op: &str, args: Vec<String>) -> Result<Value, Refusal> {
        let mut full = vec!["lessons".to_string(), op.to_string(), "--json".into()];
        full.extend(args);
        let answer = invoke(&self.root, &full, None, None)?;
        if answer.exit != 0 {
            return Err(failure(&format!("lessons {op}"), &answer));
        }
        answer.line.get("result").cloned().ok_or_else(|| {
            Refusal::new(
                "store_contract",
                format!("virgil lessons {op} answered without a result line"),
            )
        })
    }

    fn write(&self, op: &str, args: Vec<String>) -> Result<Written, Refusal> {
        let mut full = vec!["lessons".to_string(), op.to_string(), "--json".into()];
        full.extend(args);
        let via = self.via();
        let answer = invoke(&self.root, &full, None, via.as_deref())?;
        let result = &answer.line["result"];
        let id = result["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned);
        let Some(id) = id else {
            if answer.exit == 0 {
                return Err(Refusal::new(
                    "store_contract",
                    format!("virgil lessons {op} answered without a record id"),
                ));
            }
            return Err(failure(&format!("lessons {op}"), &answer));
        };
        let follow_up = (3..=5).contains(&answer.exit).then(|| {
            result["rejection"]["why"]
                .as_str()
                .filter(|why| !why.is_empty())
                .or_else(|| answer.line["error"]["message"].as_str())
                .unwrap_or("a follow-up was refused")
                .to_string()
        });
        Ok(Written {
            id,
            revision: result["revision"].as_u64().unwrap_or(0),
            trust: result["trust"].as_str().and_then(trust_of),
            via: result["via"].as_str().unwrap_or_default().to_string(),
            pending: answer.exit == 7 || result["replication_pending"] == true,
            follow_up,
        })
    }

    /// Every lesson the checkout's scope holds (all pages), or with
    /// `changed` only those touching paths changed on this branch.
    pub fn list(&self, changed: bool) -> Result<Vec<Value>, Refusal> {
        let mut lessons = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let mut args = vec![format!("--limit={PAGE}")];
            if changed {
                args.push("--changed".into());
            }
            if !cursor.is_empty() {
                args.push(format!("--cursor={cursor}"));
            }
            let page = self.read("list", args)?;
            lessons.extend(page["lessons"].as_array().cloned().ok_or_else(|| {
                Refusal::new("store_contract", "virgil lessons list returned no lessons")
            })?);
            cursor = page["cursor"].as_str().unwrap_or_default().to_string();
            if cursor.is_empty() {
                return Ok(lessons);
            }
        }
        Err(Refusal::new(
            "store_contract",
            format!("more than {MAX_PAGES} pages of lessons; the list stopped"),
        ))
    }

    /// One lesson confirmed with the authority (`--fresh`); `None` when it is
    /// missing at the confirmed cut.
    pub fn get(&self, key: &str, personal: bool) -> Result<Option<Value>, Refusal> {
        let mut args = vec!["--fresh".to_string()];
        if personal {
            args.push("--personal".into());
        }
        args.push(key.to_string());
        match self.read("get", args) {
            Ok(page) => Ok(page["lessons"]
                .as_array()
                .and_then(|lessons| lessons.first())
                .cloned()),
            Err(refusal) if refusal.code == "lesson_not_found" => Ok(None),
            Err(refusal) => Err(refusal),
        }
    }

    pub fn remember(&self, request: &Remember<'_>) -> Result<Written, Refusal> {
        let mut args = vec![
            format!("--key={}", request.key),
            format!("--text={}", request.text),
            format!("--audience={}", request.audience),
        ];
        args.extend(request.about.iter().map(|about| format!("--about={about}")));
        args.extend(
            request
                .anchors
                .iter()
                .map(|anchor| format!("--anchor={anchor}")),
        );
        if let Some(revision) = request.if_revision {
            args.push(format!("--if-revision={revision}"));
        }
        if let Some(reason) = request.reason {
            args.push(format!("--reason={reason}"));
        }
        args.push(format!("--idempotency-key={}", request.idempotency));
        self.write("remember", args)
    }

    pub fn supersede(
        &self,
        key: &str,
        successor: &str,
        if_revision: u64,
        successor_revision: u64,
        reason: &str,
        idempotency: &str,
    ) -> Result<Written, Refusal> {
        self.write(
            "supersede",
            vec![
                format!("--with={successor}"),
                format!("--if-revision={if_revision}"),
                format!("--successor-revision={successor_revision}"),
                format!("--reason={reason}"),
                format!("--idempotency-key={idempotency}"),
                key.to_string(),
            ],
        )
    }

    pub fn forget(
        &self,
        key: &str,
        if_revision: u64,
        reason: &str,
        idempotency: &str,
    ) -> Result<Written, Refusal> {
        self.write(
            "forget",
            vec![
                format!("--if-revision={if_revision}"),
                format!("--reason={reason}"),
                format!("--idempotency-key={idempotency}"),
                key.to_string(),
            ],
        )
    }
}

/// A lesson as Virgil returns it, with the pieces a rewrite passes back.
#[derive(Debug, Clone)]
pub struct WireLesson {
    pub id: String,
    pub key: String,
    pub text: String,
    pub revision: u64,
    pub status: String,
    pub trust: Option<Trust>,
    pub via: String,
    pub personal: bool,
    /// `--about` subjects: `repo` and `record:<locator>`.
    pub about: Vec<String>,
    /// `--anchor` paths, with a `#Lx-Ly` range when the anchor has one.
    pub anchors: Vec<String>,
    pub successor: String,
    /// The text is a preview; a rewrite from it would cut the lesson short.
    pub truncated: bool,
    raw: Value,
}

impl WireLesson {
    pub fn read(value: &Value) -> Result<Self, Refusal> {
        let text_of = |field: &str| value[field].as_str().unwrap_or_default().to_string();
        let key = text_of("key");
        if !crate::domain::valid_key(&key) {
            return Err(Refusal::new(
                "store_contract",
                format!("virgil returned a lesson Whetstone cannot key: {key:?}"),
            ));
        }
        let mut about = Vec::new();
        let mut anchors = Vec::new();
        for subject in value["about"].as_array().into_iter().flatten() {
            let subject = &subject["subject"];
            if subject.get("Repository").is_some() {
                about.push("repo".to_string());
            } else if let Some(locator) = subject["Locator"].as_str() {
                about.push(format!("record:{locator}"));
            } else if let Some(path) = subject["Path"]["path"].as_str() {
                let lines = match (
                    subject["Path"]["line_start"].as_i64(),
                    subject["Path"]["line_end"].as_i64(),
                ) {
                    (Some(start), Some(end)) => format!("#L{start}-L{end}"),
                    (Some(start), None) => format!("#L{start}"),
                    _ => String::new(),
                };
                anchors.push(format!("{path}{lines}"));
            }
        }
        about.dedup();
        Ok(Self {
            id: text_of("id"),
            key,
            text: text_of("text"),
            revision: value["revision"].as_u64().unwrap_or(0),
            status: text_of("status"),
            trust: value["trust"].as_str().and_then(trust_of),
            via: text_of("via"),
            personal: value["collection"] == "personal_lesson",
            about,
            anchors,
            successor: text_of("successor"),
            truncated: value["text_truncated"] == true,
            raw: value.clone(),
        })
    }

    pub fn is_current(&self) -> bool {
        matches!(self.status.as_str(), "current" | "unclassified")
    }

    /// The anchors as Virgil judged them: each path anchor's own `stale`, or
    /// every anchor when the lesson touches a path changed on this branch.
    fn anchor_state(&self, changed: bool) -> (Vec<Anchor>, Vec<AnchorComparison>) {
        let mut anchors = Vec::new();
        let mut state = Vec::new();
        for (subject, path) in self.raw["about"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|subject| subject["subject"].get("Path"))
            .zip(&self.anchors)
        {
            let digest = subject["sha256"]
                .as_str()
                .and_then(|hex| ContentDigest::new(format!("sha256:{hex}")).ok());
            anchors.push(Anchor {
                path: path.clone(),
                digest: digest.clone(),
            });
            state.push(AnchorComparison {
                path: path.clone(),
                recorded: digest,
                current: None,
                changed: changed || subject["stale"] == true,
            });
        }
        (anchors, state)
    }

    /// Whetstone's view of the lesson. A current lesson someone validated is
    /// admitted; a current lesson that is only inferred (an agent wrote it
    /// straight to Virgil) waits for a person, as a proposal does.
    pub fn stored(
        &self,
        group: &str,
        changed: bool,
        keys_by_id: &BTreeMap<String, String>,
    ) -> Result<StoredLesson, Refusal> {
        let trust = self.trust.unwrap_or(Trust::Inferred);
        let at = ulid_time(&self.id).unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
        let created_by = self.raw["created_by"].as_str().unwrap_or_default();
        let agent = self.raw["writer_kind"] == "agent" || self.via.starts_with("agent:");
        let who = match (agent, self.via.is_empty(), created_by.is_empty()) {
            (true, false, _) => self.via.clone(),
            (_, _, false) => created_by.to_string(),
            (true, true, true) => "an agent".to_string(),
            (false, _, true) => "a person".to_string(),
        };
        let optional = |field: &str| {
            self.raw[field]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let stamp = LessonStamp {
            who: who.chars().take(200).collect(),
            actor: if agent {
                ActorKind::Agent
            } else {
                ActorKind::Person
            },
            session: None,
            task: None,
            branch: optional("branch"),
            commit: optional("commit"),
            at: at.clone(),
        };
        let successor = keys_by_id
            .get(&self.successor)
            .cloned()
            .or_else(|| crate::domain::valid_key(&self.successor).then(|| self.successor.clone()))
            .filter(|successor| successor != &self.key);
        let (status, verb) = match self.status.as_str() {
            "superseded" if successor.is_some() => {
                (LessonStatus::Superseded, Some(LessonVerb::Supersede))
            }
            "superseded" | "obsolete" | "forgotten" => {
                (LessonStatus::Forgotten, Some(LessonVerb::Forget))
            }
            _ if trust == Trust::Inferred => (LessonStatus::Proposed, None),
            _ => (LessonStatus::Admitted, Some(LessonVerb::Admit)),
        };
        let decision = verb.map(|verb| LessonDecision {
            verb,
            by: stamp.who.clone(),
            at: at.clone(),
            rationale: format!(
                "{} in Virgil ({}{}).",
                self.status,
                trust.label(),
                if self.via.is_empty() {
                    String::new()
                } else {
                    format!(", via {}", self.via)
                }
            ),
            receipt: Some(format!("virgil:{}@{}", self.id, self.revision)),
        });
        let (anchors, state) = self.anchor_state(changed);
        let about = self.raw["about"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|subject| {
                let subject = &subject["subject"];
                subject["Repository"]
                    .as_str()
                    .map(|repo| Subject::Repo {
                        repo: repo.to_string(),
                    })
                    .or_else(|| {
                        subject["Locator"].as_str().map(|locator| Subject::Record {
                            locator: locator.to_string(),
                        })
                    })
            })
            .collect::<Vec<_>>();
        let lesson = Lesson {
            schema: LESSON_SCHEMA_V1.into(),
            key: self.key.clone(),
            text: self.text.clone(),
            about,
            concepts: Vec::new(),
            audience: if self.personal {
                Audience::Person {
                    name: if created_by.is_empty() {
                        "me".into()
                    } else {
                        created_by.into()
                    },
                }
            } else {
                Audience::Group { name: group.into() }
            },
            stamp,
            anchors,
            trust: if status == LessonStatus::Proposed {
                Trust::Inferred
            } else {
                trust
            },
            status,
            supersedes: None,
            superseded_by: if status == LessonStatus::Superseded {
                successor
            } else {
                None
            },
            forget_proposed: None,
            decision,
        };
        lesson.validate().map_err(|error| {
            Refusal::new(
                "store_contract",
                format!("virgil returned a lesson that breaks the contract: {error}"),
            )
        })?;
        Ok(StoredLesson {
            locator: format!("virgil:{}", self.id),
            lesson,
            revision: self.revision,
            reference: None,
            anchor_state: Some(state),
        })
    }
}

/// A Whetstone lesson's subjects and anchors as `virgil lessons remember`
/// takes them: `repo` and `record:<locator>` subjects, and every path (named
/// or added) that is a file at HEAD as an anchor. Paths gone at HEAD, or not
/// files, are dropped and returned, since Virgil hashes anchors at the
/// writer's commit and refuses anything else.
pub fn subjects(project_root: &Path, lesson: &Lesson) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut about = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for subject in &lesson.about {
        match subject {
            Subject::Repo { .. } => {
                about.insert("repo".to_string());
            }
            Subject::Record { locator } => {
                about.insert(format!("record:{locator}"));
            }
            Subject::Path { path } => {
                paths.insert(path.trim_start_matches("./").to_string());
            }
        }
    }
    paths.extend(
        lesson
            .anchors
            .iter()
            .map(|anchor| anchor.path.trim_start_matches("./").to_string()),
    );
    let mut anchors = Vec::new();
    let mut dropped = Vec::new();
    for path in paths {
        let file = path
            .split_once("#L")
            .map_or(path.as_str(), |(file, _)| file);
        let blob = Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(["cat-file", "-t", &format!("HEAD:{file}")])
            .output()
            .is_ok_and(|output| output.status.success() && output.stdout == b"blob\n");
        if blob {
            anchors.push(path);
        } else {
            dropped.push(path);
        }
    }
    let mut about = about.into_iter().collect::<Vec<_>>();
    about.truncate(MAX_SUBJECTS);
    let room = MAX_SUBJECTS - about.len();
    if anchors.len() > room {
        dropped.extend(anchors.split_off(room));
    }
    (about, anchors, dropped)
}

/// The Virgil audience for a Whetstone lesson: a personal lesson is
/// `personal`; a group lesson goes to the repository's bound channel.
pub fn audience(lesson: &Lesson) -> &'static str {
    match lesson.audience {
        Audience::Person { .. } => "personal",
        Audience::Group { .. } => "channel",
    }
}

/// Inside Virgil: admitted lessons in the group's collection, proposals (and
/// an agent's proposed forgets) in Whetstone's own store until a person
/// admits them.
pub struct VirgilLessons {
    root: PathBuf,
    group: String,
    cli: VirgilCli,
    local: BeadsLessons,
}

impl VirgilLessons {
    pub fn new(layout: &ProjectLayout, actor: ActorKind) -> Self {
        let group = super::read_config(layout.project_root())
            .ok()
            .and_then(|config| config.group().map(str::to_owned))
            .unwrap_or_default();
        Self {
            root: layout.project_root().to_path_buf(),
            group,
            cli: VirgilCli::new(layout.project_root(), actor),
            local: BeadsLessons {
                layout: layout.clone(),
            },
        }
    }

    pub fn cli(&self) -> &VirgilCli {
        &self.cli
    }

    fn virgil(&self, key: &str) -> Result<Option<(WireLesson, StoredLesson)>, Refusal> {
        let Some(value) = self.cli.get(key, false)? else {
            return Ok(None);
        };
        let wire = WireLesson::read(&value)?;
        let stored = wire.stored(&self.group, false, &BTreeMap::new())?;
        Ok(Some((wire, stored)))
    }

    /// The local proposal for `key`, if one is waiting.
    fn proposal(&self, key: &str) -> Result<Option<StoredLesson>, Refusal> {
        Ok(self
            .local
            .get(key)?
            .filter(|stored| stored.lesson.status == LessonStatus::Proposed))
    }

    /// The lesson as just written, for the response: the lesson asked for,
    /// with what Virgil derived.
    fn landed(
        &self,
        mut lesson: Lesson,
        written: &Written,
        decision: Option<LessonDecision>,
    ) -> StoredLesson {
        let trust = written.trust.unwrap_or(Trust::Validated);
        lesson.trust = trust;
        lesson.status = if trust == Trust::Inferred {
            LessonStatus::Proposed
        } else {
            LessonStatus::Admitted
        };
        lesson.decision = decision.filter(|_| lesson.status != LessonStatus::Proposed);
        lesson.supersedes = None;
        lesson.forget_proposed = None;
        StoredLesson {
            locator: format!("virgil:{}", written.id),
            lesson,
            revision: written.revision,
            reference: None,
            anchor_state: None,
        }
    }

    fn with_receipt(decision: &LessonDecision, written: &Written) -> LessonDecision {
        LessonDecision {
            receipt: Some(written.receipt()),
            ..decision.clone()
        }
    }

    /// Write `lesson` to Virgil: a rewrite of `target` in place when it
    /// exists there (revision-checked), else a new lesson under its own key.
    fn write_lesson(
        &self,
        lesson: &Lesson,
        target: Option<&WireLesson>,
        reason: &str,
        idempotency: &str,
    ) -> Result<Written, Refusal> {
        let (about, anchors, _dropped) = subjects(&self.root, lesson);
        self.cli.remember(&Remember {
            key: target.map_or(lesson.key.as_str(), |target| target.key.as_str()),
            text: &lesson.text,
            audience: audience(lesson),
            about,
            anchors,
            if_revision: target.map(|target| target.revision),
            reason: target.map(|_| reason),
            idempotency: idempotency_ulid(idempotency, millis(&lesson.stamp.at)),
        })
    }
}

impl LessonStore for VirgilLessons {
    fn backend(&self) -> &'static str {
        "virgil"
    }

    fn find(&self) -> Result<Vec<StoredLesson>, Refusal> {
        let wires = self
            .cli
            .list(false)?
            .iter()
            .map(WireLesson::read)
            .collect::<Result<Vec<_>, _>>()?;
        // Staleness is Virgil's: anchors hashed at the writer's commit, and
        // the lessons touching what this branch changed.
        let changed = self
            .cli
            .list(true)
            .map(|lessons| {
                lessons
                    .iter()
                    .filter_map(|lesson| lesson["key"].as_str().map(str::to_owned))
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let keys_by_id = wires
            .iter()
            .map(|wire| (wire.id.clone(), wire.key.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut found = BTreeMap::new();
        for wire in &wires {
            let stored = wire.stored(&self.group, changed.contains(&wire.key), &keys_by_id)?;
            found.insert(wire.key.clone(), stored);
        }
        for local in self.local.find()? {
            match found.get_mut(&local.lesson.key) {
                // An agent's proposal to forget a lesson that lives in Virgil.
                Some(remote) => {
                    if remote.lesson.status == LessonStatus::Admitted {
                        remote.lesson.forget_proposed = local.lesson.forget_proposed;
                    }
                }
                None if local.lesson.status == LessonStatus::Proposed => {
                    found.insert(local.lesson.key.clone(), local);
                }
                None => {}
            }
        }
        Ok(found.into_values().collect())
    }

    fn get(&self, key: &str) -> Result<Option<StoredLesson>, Refusal> {
        if let Some((_, mut stored)) = self.virgil(key)? {
            if stored.lesson.status == LessonStatus::Admitted {
                if let Some(local) = self.local.get(key)? {
                    stored.lesson.forget_proposed = local.lesson.forget_proposed;
                }
            }
            return Ok(Some(stored));
        }
        self.proposal(key)
    }

    /// A lesson is proposed in Whetstone's store; nothing reaches Virgil
    /// until a person admits it.
    fn remember(&self, lesson: &Lesson, idempotency: &str) -> Result<StoredLesson, Refusal> {
        if let Some((wire, _)) = self.virgil(&lesson.key)? {
            return Err(Refusal::new(
                "lesson_exists",
                format!(
                    "A lesson {} already lives in Virgil (revision {}); correct it with wh change --supersede {} instead.",
                    wire.key, wire.revision, wire.key
                ),
            ));
        }
        self.local.remember(lesson, idempotency)
    }

    /// The admit is the person's `virgil lessons remember`: a proposal is
    /// written (a correction rewrites the lesson it corrects, in place), and
    /// a current but inferred Virgil lesson is rewritten as the person so it
    /// becomes validated. Virgil derives the trust; nothing chooses it.
    fn admit(
        &self,
        key: &str,
        _trust: Trust,
        decision: &LessonDecision,
        stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        if let Some(proposal) = self.proposal(key)? {
            let corrected = match proposal.lesson.supersedes.as_deref() {
                Some(old) => self.virgil(old)?,
                None => None,
            };
            let target = corrected.as_ref().map(|(wire, _)| wire);
            if target.is_none() && self.virgil(key)?.is_some() {
                return Err(Refusal::new(
                    "lesson_exists",
                    format!("A lesson {key} already lives in Virgil; correct it with wh change --supersede {key}."),
                ));
            }
            if let Some(target) = target {
                if !target.is_current() {
                    return Err(Refusal::new(
                        "lesson_not_live",
                        format!(
                            "{} is {} in Virgil; correct the lesson that replaced it.",
                            target.key, target.status
                        ),
                    ));
                }
            }
            let written =
                self.write_lesson(&proposal.lesson, target, &decision.rationale, idempotency)?;
            let recorded = Self::with_receipt(decision, &written);
            // The proposal is settled in Whetstone's store, with Virgil's
            // receipt, so it is neither listed again nor left behind.
            let mut settled = super::decided(&proposal.lesson, recorded.clone(), stamp.clone());
            if let Some(old) = proposal.lesson.supersedes.as_deref() {
                if target.is_none() {
                    // It corrected another proposal: that one is superseded.
                    if let Some(previous) = self.proposal(old)? {
                        let mut superseded = super::decided(
                            &previous.lesson,
                            LessonDecision {
                                verb: LessonVerb::Supersede,
                                ..recorded.clone()
                            },
                            stamp.clone(),
                        );
                        superseded.superseded_by = Some(key.to_string());
                        self.local
                            .append(superseded, &format!("{idempotency}:supersedes:{old}"))?;
                    }
                }
            }
            settled.trust = written
                .trust
                .unwrap_or(Trust::Validated)
                .max(Trust::Validated);
            self.local
                .append(settled, &format!("{idempotency}:settled"))?;
            let mut lesson = proposal.lesson.clone();
            if let Some(target) = target {
                lesson.key = target.key.clone();
            }
            return Ok(self.landed(lesson, &written, Some(recorded)));
        }
        let Some((wire, stored)) = self.virgil(key)? else {
            return Err(Refusal::new(
                "lesson_not_found",
                format!("No lesson {key} exists; see wh dash --json (current.lessons)."),
            ));
        };
        if stored.lesson.status != LessonStatus::Proposed {
            return Err(Refusal::new(
                "lesson_not_proposed",
                format!(
                    "{key} is {} in Virgil, not waiting on a person; nothing to admit.",
                    stored.lesson.status.label()
                ),
            ));
        }
        if wire.truncated {
            return Err(Refusal::new(
                "store_contract",
                format!("virgil returned only a preview of {key}'s text, so it cannot be rewritten as yours; admit it in Virgil instead."),
            ));
        }
        // Rewrite it as the person, keeping its subjects and anchors.
        let written = self.cli.remember(&Remember {
            key,
            text: &wire.text,
            audience: if wire.personal { "personal" } else { "channel" },
            about: wire.about.clone(),
            anchors: wire.anchors.clone(),
            if_revision: Some(wire.revision),
            reason: Some(&decision.rationale),
            idempotency: idempotency_ulid(idempotency, millis(&stamp.at)),
        })?;
        let recorded = Self::with_receipt(decision, &written);
        Ok(self.landed(stored.lesson, &written, Some(recorded)))
    }

    /// A correction is a proposal in Whetstone's store; admitting it
    /// rewrites the corrected lesson in place.
    fn supersede(
        &self,
        key: &str,
        correction: &Lesson,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        let Some(old) = self.get(key)? else {
            return Err(Refusal::new(
                "lesson_not_found",
                format!("No lesson {key} exists to correct."),
            ));
        };
        if matches!(
            old.lesson.status,
            LessonStatus::Superseded | LessonStatus::Forgotten
        ) {
            return Err(Refusal::new(
                "lesson_not_live",
                format!(
                    "{key} is {}; correct the lesson that replaced it.",
                    old.lesson.status.label()
                ),
            ));
        }
        self.remember(correction, idempotency)
    }

    fn forget(
        &self,
        key: &str,
        decision: Option<&LessonDecision>,
        proposal: Option<&ForgetProposal>,
        stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        let Some((wire, stored)) = self.virgil(key)? else {
            return self
                .local
                .forget(key, decision, proposal, stamp, idempotency);
        };
        match (decision, proposal) {
            (Some(decision), _) => {
                if !wire.is_current() {
                    return Err(Refusal::new(
                        "lesson_not_live",
                        format!("{key} is already {} in Virgil.", wire.status),
                    ));
                }
                let written = self.cli.forget(
                    key,
                    wire.revision,
                    &decision.rationale,
                    &idempotency_ulid(idempotency, millis(&stamp.at)),
                )?;
                let mut lesson = super::decided(
                    &stored.lesson,
                    Self::with_receipt(decision, &written),
                    stamp.clone(),
                );
                lesson.forget_proposed = None;
                Ok(StoredLesson {
                    locator: format!("virgil:{}", written.id),
                    lesson,
                    revision: written.revision,
                    reference: None,
                    anchor_state: None,
                })
            }
            // An agent's proposed forget waits in Whetstone's store beside a
            // copy of the lesson as read.
            (None, Some(proposal)) => {
                let mut copy = stored.lesson.clone();
                copy.stamp = stamp.clone();
                copy.forget_proposed = Some(proposal.clone());
                self.local.append(copy, idempotency)?;
                let mut lesson = stored.lesson;
                lesson.forget_proposed = Some(proposal.clone());
                Ok(StoredLesson { lesson, ..stored })
            }
            (None, None) => Err(Refusal::new(
                "invalid_request",
                "Forget needs a decision or a proposal.",
            )),
        }
    }

    fn retire(
        &self,
        key: &str,
        successor: &str,
        decision: &LessonDecision,
        idempotency: &str,
    ) -> Result<Option<StoredLesson>, Refusal> {
        let (Some((old, stored)), Some((next, _))) = (self.virgil(key)?, self.virgil(successor)?)
        else {
            return Ok(None);
        };
        if !old.is_current() || !next.is_current() {
            return Err(Refusal::new(
                "lesson_not_live",
                format!(
                    "{key} is {} and {successor} is {} in Virgil; both must be current.",
                    old.status, next.status
                ),
            ));
        }
        let written = self.cli.supersede(
            key,
            successor,
            old.revision,
            next.revision,
            &decision.rationale,
            &idempotency_ulid(idempotency, millis(&decision.at)),
        )?;
        let mut lesson = stored.lesson;
        lesson.status = LessonStatus::Superseded;
        lesson.superseded_by = Some(successor.to_string());
        lesson.decision = Some(LessonDecision {
            verb: LessonVerb::Supersede,
            ..Self::with_receipt(decision, &written)
        });
        Ok(Some(StoredLesson {
            locator: format!("virgil:{}", written.id),
            lesson,
            revision: written.revision,
            reference: None,
            anchor_state: None,
        }))
    }

    fn taken_keys(&self) -> Result<BTreeSet<String>, Refusal> {
        let mut keys = self
            .find()?
            .into_iter()
            .map(|stored| stored.lesson.key)
            .collect::<BTreeSet<_>>();
        keys.extend(
            self.local
                .find()?
                .into_iter()
                .map(|stored| stored.lesson.key),
        );
        Ok(keys)
    }

    /// Lessons admitted in Whetstone's own store before the switch, not yet
    /// in Virgil (an import moves them; until then `wh dash` reports them).
    fn left_behind(&self, current: &[StoredLesson]) -> Result<Vec<String>, Refusal> {
        let here = current
            .iter()
            .filter(|stored| stored.locator.starts_with("virgil:"))
            .map(|stored| stored.lesson.key.as_str())
            .collect::<BTreeSet<_>>();
        Ok(self
            .local
            .find()?
            .into_iter()
            .filter(|stored| {
                matches!(
                    stored.lesson.status,
                    LessonStatus::Admitted | LessonStatus::Superseded
                ) && !stored
                    .lesson
                    .decision
                    .as_ref()
                    .and_then(|decision| decision.receipt.as_deref())
                    .is_some_and(|receipt| receipt.starts_with("virgil:"))
                    && !here.contains(stored.lesson.key.as_str())
            })
            .map(|stored| stored.lesson.key)
            .collect())
    }
}

/// The repository this checkout is attached to (`virgil repos show`), as
/// `(group, repository)`; `None` when it is not attached.
pub fn attachment(project_root: &Path) -> Result<Option<(String, String)>, Refusal> {
    let answer = invoke(
        project_root,
        &["repos".to_string(), "show".to_string()],
        None,
        None,
    )?;
    if answer.exit != 0 {
        let refusal = failure("repos show", &answer);
        return if refusal.code == "unattached" {
            Ok(None)
        } else {
            Err(refusal)
        };
    }
    let identity = &answer.line["result"]["resolution"]["repository"]["identity"];
    Ok(
        match (identity["group"].as_str(), identity["repository"].as_str()) {
            (Some(group), Some(repository)) if !group.is_empty() && !repository.is_empty() => {
                Some((group.to_string(), repository.to_string()))
            }
            _ => None,
        },
    )
}

/// Run one `virgil tasks` operation and return its `result`. `virgil tasks`
/// does not read `VIRGIL_VIA` (a task is the daemon's principal's whoever
/// asked), so none is passed; an agent's hand names the agent in the task.
pub fn call(
    project_root: &Path,
    family: &str,
    op: &str,
    group: &str,
    args: &[&str],
    request: &Value,
) -> Result<Value, Refusal> {
    let mut full = vec![family.to_string(), op.to_string()];
    full.extend(args.iter().map(|arg| (*arg).to_string()));
    full.extend([
        "--group".into(),
        group.to_string(),
        "--input".into(),
        "-".into(),
    ]);
    let answer = invoke(project_root, &full, Some(&request.to_string()), None)?;
    if answer.exit != 0 || answer.line.get("error").is_some() {
        return Err(failure(&format!("{family} {op}"), &answer));
    }
    answer.line.get("result").cloned().ok_or_else(|| {
        Refusal::new(
            "store_contract",
            format!("virgil {family} {op} answered without a result line"),
        )
    })
}

/// File a raised hand as a native task in the group's channel; returns the
/// task reference (`task:<ULID>`).
pub fn create_task(
    project_root: &Path,
    group: &str,
    title: &str,
    description: &str,
) -> Result<String, Refusal> {
    let result = call(
        project_root,
        "tasks",
        "create",
        group,
        &[],
        &json!({"scope": {"group": group}, "title": title, "description": description}),
    )?;
    result["task"]
        .as_str()
        .filter(|task| !task.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Refusal::new("store_contract", "virgil tasks create returned no task"))
}

/// A task's state and its latest comment, from this device's replica
/// (`show` answers `{"task":{...,"state"},"comments":[{"text"}]}`).
pub fn task_status(
    project_root: &Path,
    group: &str,
    task: &str,
) -> Option<(String, Option<String>)> {
    let result = call(project_root, "tasks", "show", group, &[task], &json!({})).ok()?;
    let detail = result
        .get("task")
        .filter(|task| task.is_object())
        .unwrap_or(&result);
    let state = detail["state"].as_str()?.to_string();
    let answer = result["comments"]
        .as_array()
        .or_else(|| detail["comments"].as_array())
        .and_then(|comments| comments.last())
        .and_then(|comment| {
            comment["text"]
                .as_str()
                .or_else(|| comment["body"].as_str())
        })
        .map(str::to_owned);
    Some((state, answer))
}

/// Answer a raised hand's task: the answer as a comment, then done.
pub fn answer_task(
    project_root: &Path,
    group: &str,
    task: &str,
    answer: &str,
) -> Result<(), Refusal> {
    call(
        project_root,
        "tasks",
        "comment",
        group,
        &[task],
        &json!({"task": task, "text": answer}),
    )?;
    call(
        project_root,
        "tasks",
        "move",
        group,
        &[task],
        &json!({"task": task, "state": "done"}),
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(trust: &str, status: &str) -> Value {
        json!({
            "id": "01J8Z0000000000000000000L1",
            "key": "values-are-floats",
            "text": "Values are floats.",
            "revision": 3,
            "status": status,
            "trust": trust,
            "via": if trust == "inferred" { "agent:codex" } else { "" },
            "writer_kind": if trust == "inferred" { "agent" } else { "person" },
            "created_by": "principal:P",
            "collection": "lesson",
            "about": [
                {"subject": {"Repository": "repository:R"}},
                {"subject": {"Locator": "beads:whetstone-jpd"}},
                {"subject": {"Path": {"repository": "repository:R", "path": "app.py", "sha256": "a".repeat(64), "line_start": 3, "line_end": 9, "stale": true}}},
            ],
        })
    }

    #[test]
    fn a_validated_current_lesson_is_admitted_and_keeps_what_a_rewrite_needs() {
        let lesson = WireLesson::read(&wire("validated", "current")).expect("wire");
        assert_eq!(lesson.about, ["repo", "record:beads:whetstone-jpd"]);
        assert_eq!(lesson.anchors, ["app.py#L3-L9"]);
        let stored = lesson.stored("G", false, &BTreeMap::new()).expect("stored");
        assert_eq!(stored.lesson.status, LessonStatus::Admitted);
        assert_eq!(stored.lesson.trust, Trust::Validated);
        assert_eq!(stored.locator, "virgil:01J8Z0000000000000000000L1");
        let state = stored.anchor_state.expect("virgil judged the anchors");
        assert!(state[0].changed, "Virgil said the anchor is stale");
        assert_eq!(
            stored.lesson.anchors[0]
                .digest
                .as_ref()
                .map(ContentDigest::as_str),
            Some(format!("sha256:{}", "a".repeat(64)).as_str())
        );
    }

    #[test]
    fn a_current_inferred_lesson_waits_on_a_person() {
        let stored = WireLesson::read(&wire("inferred", "current"))
            .expect("wire")
            .stored("G", false, &BTreeMap::new())
            .expect("stored");
        assert_eq!(stored.lesson.status, LessonStatus::Proposed);
        assert_eq!(stored.lesson.stamp.who, "agent:codex");
        assert_eq!(stored.lesson.stamp.actor, ActorKind::Agent);
    }

    #[test]
    fn idempotency_keys_are_ulids_and_repeat_for_the_same_request() {
        let at = millis("2026-10-14T09:00:00Z");
        let key = idempotency_ulid("lesson:admit:r1:k", at);
        assert_eq!(key, idempotency_ulid("lesson:admit:r1:k", at));
        assert_ne!(key, idempotency_ulid("lesson:admit:r2:k", at));
        assert_eq!(key.len(), 26);
        assert!(key.as_bytes()[0] <= b'7');
        assert_eq!(ulid_time(&key).as_deref(), Some("2026-10-14T09:00:00Z"));
    }
}
