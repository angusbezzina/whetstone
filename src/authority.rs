//! Team authority over the platform trust root (ADR-0002): who may propose,
//! approve and activate shared policy, verified from GitHub evidence and
//! never from self-declared fields.
//!
//! The flow: `wh push --propose` shares accepted records, records a shared
//! proposal bound to the exact payload, base and authority revision, and
//! writes a small activation manifest (`.whetstone/proposals/<id>.json`) for
//! a pull request. GitHub's protected review is the independent approval.
//! After the manifest merges, a CI activator runs
//! `wh change --activate <manifest>`: it re-fetches the platform evidence,
//! verifies every binding here, and only then appends the decision and
//! activation records. Any mismatch is a denial with a reason code; nothing
//! is ever activated on unverifiable evidence.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MANIFEST_SCHEMA: &str = "whetstone.activation-proposal.v1";
pub const AUTHORITY_SCHEMA: &str = "whetstone.authority.v1";
pub const AUTHORITY_PATH: &str = ".whetstone/authority.json";
pub const PROPOSALS_DIR: &str = ".whetstone/proposals";
/// The required status check that runs `wh check --required`.
pub const REQUIRED_CHECK: &str = "whetstone/policy";

/// The reviewable activation instruction. Unknown fields (an `approved_by`,
/// say) make it invalid rather than being ignored silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationManifest {
    pub schema: String,
    pub repository_id: u64,
    pub proposal_id: String,
    pub proposal_revision: u64,
    pub proposal_digest: String,
    pub payload_digest: String,
    pub base_active_digest: String,
    pub scope: Vec<String>,
    pub authority_revision: u64,
    pub expires_at: String,
    /// The exact record revisions proposed (`id@revision#digest`).
    pub records: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Proposer,
    Reviewer,
    Maintainer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityPrincipal {
    /// Immutable numeric GitHub user id; logins are informational only.
    pub github_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    pub roles: Vec<Role>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// The protected, versioned authority file (`.whetstone/authority.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub schema: String,
    pub repository_id: u64,
    pub revision: u64,
    pub principals: Vec<AuthorityPrincipal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked: Vec<u64>,
    /// Workflow files allowed to activate (`.github/workflows/<file>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activator_workflows: Vec<String>,
}

impl Authority {
    pub fn load(project_root: &Path) -> Result<Self, Denial> {
        let path = project_root.join(AUTHORITY_PATH);
        let bytes = std::fs::read(&path).map_err(|_| {
            Denial::new(
                "authority_missing",
                format!("{AUTHORITY_PATH} does not exist; team activation is unsupported until the protected authority file is committed."),
            )
        })?;
        let authority: Self = serde_json::from_slice(&bytes).map_err(|error| {
            Denial::new("authority_invalid", format!("{AUTHORITY_PATH}: {error}"))
        })?;
        if authority.schema != AUTHORITY_SCHEMA || authority.revision == 0 {
            return Err(Denial::new(
                "authority_invalid",
                format!(
                    "{AUTHORITY_PATH} must use schema {AUTHORITY_SCHEMA} and a positive revision."
                ),
            ));
        }
        Ok(authority)
    }

    fn principal(&self, github_id: u64, now: &str) -> Option<&AuthorityPrincipal> {
        if self.revoked.contains(&github_id) {
            return None;
        }
        self.principals.iter().find(|principal| {
            principal.github_id == github_id
                && principal
                    .expires_at
                    .as_deref()
                    .map_or(true, |expires| expires > now)
        })
    }

    pub fn may(&self, github_id: u64, role: Role, now: &str) -> bool {
        self.principal(github_id, now).is_some_and(|principal| {
            principal.roles.contains(&role) || principal.roles.contains(&Role::Maintainer)
        })
    }

    fn covers_scope(&self, github_id: u64, scope: &[String], now: &str) -> bool {
        self.principal(github_id, now).is_some_and(|principal| {
            principal.scopes.is_empty()
                || scope.iter().all(|wanted| {
                    principal
                        .scopes
                        .iter()
                        .any(|granted| wanted.starts_with(granted.as_str()))
                })
        })
    }
}

/// A refusal with a stable reason code (ADR-0002's fixture names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Denial {
    pub code: &'static str,
    pub detail: String,
}

impl Denial {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Denial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub github_id: u64,
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub reviewer_id: u64,
    pub state: String,
    pub commit_id: String,
}

/// What the platform says about the pull request that merged a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeEvidence {
    pub repository_id: u64,
    pub pull_request: u64,
    pub merged: bool,
    pub base_ref: String,
    pub head_sha: String,
    pub merge_commit_sha: String,
    pub author_id: u64,
    pub reviews: Vec<Review>,
    pub changed_files: Vec<String>,
}

/// The branch rules actually in effect, as the platform reports them.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProtectionEvidence {
    pub required_approvals: u32,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_review: bool,
    pub require_last_push_approval: bool,
    pub required_checks: Vec<String>,
    pub strict_required_checks: bool,
    pub allows_force_pushes: bool,
    pub allows_deletions: bool,
}

impl ProtectionEvidence {
    /// Every protection ADR-0002 requires, as a list of what is missing.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.required_approvals == 0 {
            missing.push("a required approving review");
        }
        if !self.dismiss_stale_reviews {
            missing.push("dismissal of stale approvals");
        }
        if !self.require_code_owner_review {
            missing.push("code owner review of .whetstone/**");
        }
        if !self.require_last_push_approval {
            missing.push("approval of the most recent push by someone else");
        }
        if !self
            .required_checks
            .iter()
            .any(|check| check == REQUIRED_CHECK)
        {
            missing.push("the whetstone/policy required status check");
        }
        if !self.strict_required_checks {
            missing.push("up-to-date branches before merge");
        }
        if self.allows_force_pushes {
            missing.push("no force pushes");
        }
        if self.allows_deletions {
            missing.push("no branch deletion");
        }
        missing
    }
}

/// The trust root's read interface. The GitHub implementation shells out to
/// `gh api`; tests supply fixtures.
pub trait Platform {
    fn actor(&self) -> Result<Actor, Denial>;
    fn repository_id(&self) -> Result<u64, Denial>;
    fn merge_evidence(&self, commit: &str) -> Result<MergeEvidence, Denial>;
    fn protection(&self, branch: &str) -> Result<ProtectionEvidence, Denial>;
}

/// What a verified activation may append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifiedActivation {
    pub reviewer_id: u64,
    pub pull_request: u64,
    pub merge_commit_sha: String,
    pub head_sha: String,
}

/// Everything the activator checked, in one place, before any effect.
pub struct ActivationInputs<'a> {
    pub manifest: &'a ActivationManifest,
    pub manifest_path: &'a str,
    pub authority: &'a Authority,
    pub merge: &'a MergeEvidence,
    pub protection: &'a ProtectionEvidence,
    /// Proposal owner (GitHub id) as recorded in the shared proposal.
    pub proposer_id: u64,
    /// Digest of the proposal record actually in the shared store.
    pub stored_proposal_digest: &'a str,
    /// Digest of the proposed records actually in the shared store.
    pub stored_payload_digest: &'a str,
    /// The team-active digest right now.
    pub current_active_digest: &'a str,
    pub now: &'a str,
}

/// Verify an activation against platform evidence. Fails closed.
pub fn verify_activation(input: &ActivationInputs<'_>) -> Result<VerifiedActivation, Denial> {
    let manifest = input.manifest;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(Denial::new(
            "manifest_invalid",
            format!("unsupported schema {}", manifest.schema),
        ));
    }
    if manifest.repository_id != input.authority.repository_id
        || input.merge.repository_id != manifest.repository_id
    {
        return Err(Denial::new(
            "repository_mismatch",
            "the manifest, the authority file and the pull request name different repositories",
        ));
    }
    if manifest.expires_at.as_str() <= input.now {
        return Err(Denial::new(
            "expired",
            format!("the proposal expired at {}", manifest.expires_at),
        ));
    }
    if manifest.authority_revision != input.authority.revision {
        return Err(Denial::new(
            "authority_stale",
            format!(
                "the proposal was bound to authority revision {} but revision {} is in force; propose again",
                manifest.authority_revision, input.authority.revision
            ),
        ));
    }
    if !input.merge.merged {
        return Err(Denial::new(
            "not_merged",
            "the activation pull request is not merged",
        ));
    }
    if !input
        .merge
        .changed_files
        .iter()
        .any(|file| file == input.manifest_path)
    {
        return Err(Denial::new(
            "manifest_not_reviewed",
            format!(
                "pull request #{} did not change {}",
                input.merge.pull_request, input.manifest_path
            ),
        ));
    }
    let missing = input.protection.missing();
    if !missing.is_empty() {
        return Err(Denial::new(
            "protection_unverified",
            format!(
                "the default branch does not enforce: {}",
                missing.join(", ")
            ),
        ));
    }
    if input.stored_proposal_digest != manifest.proposal_digest {
        return Err(Denial::new(
            "digest_mismatch",
            "the shared proposal differs from the one the manifest binds",
        ));
    }
    if input.stored_payload_digest != manifest.payload_digest {
        return Err(Denial::new(
            "digest_mismatch",
            "the shared records differ from the reviewed payload",
        ));
    }
    if input.current_active_digest != manifest.base_active_digest {
        return Err(Denial::new(
            "stale_base",
            "team-active policy changed since the proposal was reviewed; rebase and review again",
        ));
    }
    if !input
        .authority
        .may(input.proposer_id, Role::Proposer, input.now)
    {
        return Err(Denial::new(
            "unauthorized_proposer",
            format!(
                "GitHub user {} may not propose policy under this authority revision",
                input.proposer_id
            ),
        ));
    }
    if !input
        .authority
        .covers_scope(input.proposer_id, &manifest.scope, input.now)
    {
        return Err(Denial::new(
            "scope_mismatch",
            "the proposal reaches beyond the proposer's scopes",
        ));
    }
    // The independent approval: an authorized reviewer, not the proposer and
    // not the pull request's author, approving the exact merged head.
    let reviewer = input.merge.reviews.iter().find(|review| {
        review.state.eq_ignore_ascii_case("APPROVED")
            && review.commit_id == input.merge.head_sha
            && review.reviewer_id != input.proposer_id
            && review.reviewer_id != input.merge.author_id
            && input
                .authority
                .may(review.reviewer_id, Role::Reviewer, input.now)
            && input
                .authority
                .covers_scope(review.reviewer_id, &manifest.scope, input.now)
    });
    let Some(reviewer) = reviewer else {
        let self_review = input.merge.reviews.iter().any(|review| {
            review.state.eq_ignore_ascii_case("APPROVED")
                && (review.reviewer_id == input.proposer_id
                    || review.reviewer_id == input.merge.author_id)
        });
        let stale = input.merge.reviews.iter().any(|review| {
            review.state.eq_ignore_ascii_case("APPROVED")
                && review.commit_id != input.merge.head_sha
        });
        return Err(if self_review {
            Denial::new(
                "self_review",
                "the only approval came from the proposer or the pull request author",
            )
        } else if stale {
            Denial::new(
                "stale_review",
                "approvals were given on an older commit than the merged head",
            )
        } else {
            Denial::new(
                "no_independent_approval",
                "no authorized, independent reviewer approved the merged head",
            )
        });
    };
    Ok(VerifiedActivation {
        reviewer_id: reviewer.reviewer_id,
        pull_request: input.merge.pull_request,
        merge_commit_sha: input.merge.merge_commit_sha.clone(),
        head_sha: input.merge.head_sha.clone(),
    })
}

/// The platform the CLI uses: GitHub through `gh`. Debug builds (tests
/// only; never distributed) may read recorded platform answers from the file
/// named by `WH_TEST_PLATFORM_FIXTURE`, so the binary's required-check and
/// activation paths can be exercised without network access. Release builds
/// ignore that variable entirely.
pub fn platform_for(root: &Path) -> Box<dyn Platform> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("WH_TEST_PLATFORM_FIXTURE") {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(fixture) = serde_json::from_slice::<FixturePlatform>(&bytes) {
                return Box::new(fixture);
            }
        }
    }
    Box::new(GhPlatform::new(root))
}

/// Recorded platform answers for tests (debug builds only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixturePlatform {
    pub actor: Actor,
    pub repository_id: u64,
    pub merges: std::collections::BTreeMap<String, MergeEvidence>,
    pub protection: ProtectionEvidence,
}

impl Platform for FixturePlatform {
    fn actor(&self) -> Result<Actor, Denial> {
        Ok(self.actor.clone())
    }
    fn repository_id(&self) -> Result<u64, Denial> {
        Ok(self.repository_id)
    }
    fn merge_evidence(&self, commit: &str) -> Result<MergeEvidence, Denial> {
        self.merges.get(commit).cloned().ok_or_else(|| {
            Denial::new(
                "not_merged",
                format!("no merged pull request produced {commit}"),
            )
        })
    }
    fn protection(&self, _branch: &str) -> Result<ProtectionEvidence, Denial> {
        Ok(self.protection.clone())
    }
}

/// GitHub through the `gh` CLI (authenticated by the user's token locally
/// or `GITHUB_TOKEN`/`GH_TOKEN` in Actions).
pub struct GhPlatform {
    root: std::path::PathBuf,
}

impl GhPlatform {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn api(&self, path: &str) -> Result<Value, Denial> {
        let output = Command::new("gh")
            .args(["api", "-H", "Accept: application/vnd.github+json", path])
            .current_dir(&self.root)
            .output()
            .map_err(|error| {
                Denial::new(
                    "platform_unavailable",
                    format!("gh could not start: {error}"),
                )
            })?;
        if !output.status.success() {
            return Err(Denial::new(
                "platform_unavailable",
                format!(
                    "gh api {path}: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .next()
                        .unwrap_or("failed")
                ),
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| Denial::new("platform_unavailable", format!("gh api {path}: {error}")))
    }

    fn repository(&self) -> Result<String, Denial> {
        if let Ok(repository) = std::env::var("GITHUB_REPOSITORY") {
            if repository.contains('/') {
                return Ok(repository);
            }
        }
        let output = Command::new("gh")
            .args([
                "repo",
                "view",
                "--json",
                "nameWithOwner",
                "--jq",
                ".nameWithOwner",
            ])
            .current_dir(&self.root)
            .output()
            .map_err(|error| Denial::new("platform_unavailable", error.to_string()))?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() && name.contains('/') {
            Ok(name)
        } else {
            Err(Denial::new(
                "platform_unavailable",
                "the GitHub repository could not be resolved",
            ))
        }
    }
}

fn number(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn flag(value: &Value, pointer: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

impl Platform for GhPlatform {
    fn actor(&self) -> Result<Actor, Denial> {
        let user = self.api("user")?;
        let github_id = number(&user, "/id");
        if github_id == 0 {
            return Err(Denial::new(
                "missing_identity",
                "the authenticated GitHub user has no numeric id",
            ));
        }
        Ok(Actor {
            github_id,
            login: user
                .get("login")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    fn repository_id(&self) -> Result<u64, Denial> {
        let repository = self.repository()?;
        let id = number(&self.api(&format!("repos/{repository}"))?, "/id");
        if id == 0 {
            Err(Denial::new(
                "platform_unavailable",
                "the repository has no numeric id",
            ))
        } else {
            Ok(id)
        }
    }

    fn merge_evidence(&self, commit: &str) -> Result<MergeEvidence, Denial> {
        if !commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
            || commit.len() < 7
        {
            return Err(Denial::new("manifest_invalid", "invalid commit"));
        }
        let repository = self.repository()?;
        let repository_id = self.repository_id()?;
        let pulls = self.api(&format!("repos/{repository}/commits/{commit}/pulls"))?;
        let pull = pulls
            .as_array()
            .and_then(|pulls| {
                pulls.iter().find(|pull| {
                    pull.get("merge_commit_sha").and_then(Value::as_str) == Some(commit)
                })
            })
            .ok_or_else(|| {
                Denial::new(
                    "not_merged",
                    format!("no merged pull request produced {commit}"),
                )
            })?;
        let number_ = number(pull, "/number");
        let detail = self.api(&format!("repos/{repository}/pulls/{number_}"))?;
        let reviews = self.api(&format!(
            "repos/{repository}/pulls/{number_}/reviews?per_page=100"
        ))?;
        let files = self.api(&format!(
            "repos/{repository}/pulls/{number_}/files?per_page=100"
        ))?;
        Ok(MergeEvidence {
            repository_id,
            pull_request: number_,
            merged: flag(&detail, "/merged"),
            base_ref: detail
                .pointer("/base/ref")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            head_sha: detail
                .pointer("/head/sha")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            merge_commit_sha: detail
                .get("merge_commit_sha")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            author_id: number(&detail, "/user/id"),
            reviews: reviews
                .as_array()
                .map(|reviews| {
                    reviews
                        .iter()
                        .map(|review| Review {
                            reviewer_id: number(review, "/user/id"),
                            state: review
                                .get("state")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            commit_id: review
                                .get("commit_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            changed_files: files
                .as_array()
                .map(|files| {
                    files
                        .iter()
                        .filter_map(|file| {
                            file.get("filename")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Rulesets are readable with read access; classic branch protection
    /// needs admin. Either proves the rules; neither means unverified.
    fn protection(&self, branch: &str) -> Result<ProtectionEvidence, Denial> {
        let repository = self.repository()?;
        if let Ok(rules) = self.api(&format!("repos/{repository}/rules/branches/{branch}")) {
            let rules = rules.as_array().cloned().unwrap_or_default();
            if !rules.is_empty() {
                let mut evidence = ProtectionEvidence {
                    allows_force_pushes: true,
                    allows_deletions: true,
                    ..ProtectionEvidence::default()
                };
                for rule in &rules {
                    match rule.get("type").and_then(Value::as_str).unwrap_or_default() {
                        "pull_request" => {
                            evidence.required_approvals = u32::try_from(number(
                                rule,
                                "/parameters/required_approving_review_count",
                            ))
                            .unwrap_or(0);
                            evidence.dismiss_stale_reviews =
                                flag(rule, "/parameters/dismiss_stale_reviews_on_push");
                            evidence.require_code_owner_review =
                                flag(rule, "/parameters/require_code_owner_review");
                            evidence.require_last_push_approval =
                                flag(rule, "/parameters/require_last_push_approval");
                        }
                        "required_status_checks" => {
                            evidence.strict_required_checks =
                                flag(rule, "/parameters/strict_required_status_checks_policy");
                            evidence.required_checks = rule
                                .pointer("/parameters/required_status_checks")
                                .and_then(Value::as_array)
                                .map(|checks| {
                                    checks
                                        .iter()
                                        .filter_map(|check| {
                                            check
                                                .get("context")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned)
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                        }
                        "non_fast_forward" => evidence.allows_force_pushes = false,
                        "deletion" => evidence.allows_deletions = false,
                        _ => {}
                    }
                }
                return Ok(evidence);
            }
        }
        let protection = self
            .api(&format!("repos/{repository}/branches/{branch}/protection"))
            .map_err(|denial| {
                Denial::new(
                    "protection_unverified",
                    format!("branch rules for {branch} could not be read ({}); team activation is unsupported until they can be", denial.detail),
                )
            })?;
        Ok(ProtectionEvidence {
            required_approvals: u32::try_from(number(
                &protection,
                "/required_pull_request_reviews/required_approving_review_count",
            ))
            .unwrap_or(0),
            dismiss_stale_reviews: flag(
                &protection,
                "/required_pull_request_reviews/dismiss_stale_reviews",
            ),
            require_code_owner_review: flag(
                &protection,
                "/required_pull_request_reviews/require_code_owner_reviews",
            ),
            require_last_push_approval: flag(
                &protection,
                "/required_pull_request_reviews/require_last_push_approval",
            ),
            required_checks: protection
                .pointer("/required_status_checks/contexts")
                .and_then(Value::as_array)
                .map(|checks| {
                    checks
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            strict_required_checks: flag(&protection, "/required_status_checks/strict"),
            allows_force_pushes: flag(&protection, "/allow_force_pushes/enabled"),
            allows_deletions: flag(&protection, "/allow_deletions/enabled"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn authority() -> Authority {
        Authority {
            schema: AUTHORITY_SCHEMA.into(),
            repository_id: 42,
            revision: 3,
            principals: vec![
                AuthorityPrincipal {
                    github_id: 100,
                    login: Some("ada".into()),
                    roles: vec![Role::Proposer],
                    scopes: Vec::new(),
                    expires_at: None,
                },
                AuthorityPrincipal {
                    github_id: 200,
                    login: Some("grace".into()),
                    roles: vec![Role::Reviewer],
                    scopes: Vec::new(),
                    expires_at: None,
                },
                AuthorityPrincipal {
                    github_id: 300,
                    login: Some("linus".into()),
                    roles: vec![Role::Reviewer],
                    scopes: vec!["docs".into()],
                    expires_at: None,
                },
            ],
            revoked: Vec::new(),
            activator_workflows: vec!["whetstone-policy.yml".into()],
        }
    }

    fn manifest() -> ActivationManifest {
        ActivationManifest {
            schema: MANIFEST_SCHEMA.into(),
            repository_id: 42,
            proposal_id: "proposal.team_x".into(),
            proposal_revision: 1,
            proposal_digest: "sha256:proposal".into(),
            payload_digest: "sha256:payload".into(),
            base_active_digest: "sha256:base".into(),
            scope: vec!["project".into()],
            authority_revision: 3,
            expires_at: "2026-09-17T00:00:00Z".into(),
            records: vec!["standard.x@1#sha256:y".into()],
        }
    }

    fn merge(reviews: Vec<Review>) -> MergeEvidence {
        MergeEvidence {
            repository_id: 42,
            pull_request: 7,
            merged: true,
            base_ref: "main".into(),
            head_sha: HEAD.into(),
            merge_commit_sha: "b".repeat(40),
            author_id: 100,
            reviews,
            changed_files: vec![".whetstone/proposals/proposal.team_x.json".into()],
        }
    }

    fn protected() -> ProtectionEvidence {
        ProtectionEvidence {
            required_approvals: 1,
            dismiss_stale_reviews: true,
            require_code_owner_review: true,
            require_last_push_approval: true,
            required_checks: vec![REQUIRED_CHECK.into()],
            strict_required_checks: true,
            allows_force_pushes: false,
            allows_deletions: false,
        }
    }

    fn approve(reviewer: u64, commit: &str) -> Review {
        Review {
            reviewer_id: reviewer,
            state: "APPROVED".into(),
            commit_id: commit.into(),
        }
    }

    fn verify(
        manifest: &ActivationManifest,
        authority: &Authority,
        merge: &MergeEvidence,
        protection: &ProtectionEvidence,
        payload: &str,
        active: &str,
    ) -> Result<VerifiedActivation, Denial> {
        verify_activation(&ActivationInputs {
            manifest,
            manifest_path: ".whetstone/proposals/proposal.team_x.json",
            authority,
            merge,
            protection,
            proposer_id: 100,
            stored_proposal_digest: "sha256:proposal",
            stored_payload_digest: payload,
            current_active_digest: active,
            now: "2026-09-10T00:00:00Z",
        })
    }

    #[test]
    fn an_independent_approval_of_the_exact_head_activates() {
        let verified = verify(
            &manifest(),
            &authority(),
            &merge(vec![approve(200, HEAD)]),
            &protected(),
            "sha256:payload",
            "sha256:base",
        )
        .expect("verified");
        assert_eq!(verified.reviewer_id, 200);
    }

    #[test]
    fn every_adr_fixture_fails_closed_with_its_reason() {
        let base = |reviews| merge(reviews);
        let cases: Vec<(&str, Result<VerifiedActivation, Denial>)> = vec![
            (
                "self_review",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(100, HEAD)]),
                    &protected(),
                    "sha256:payload",
                    "sha256:base",
                ),
            ),
            (
                "stale_review",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(200, "c0ffee")]),
                    &protected(),
                    "sha256:payload",
                    "sha256:base",
                ),
            ),
            (
                "no_independent_approval",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(999, HEAD)]),
                    &protected(),
                    "sha256:payload",
                    "sha256:base",
                ),
            ),
            (
                "digest_mismatch",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(200, HEAD)]),
                    &protected(),
                    "sha256:changed",
                    "sha256:base",
                ),
            ),
            (
                "stale_base",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(200, HEAD)]),
                    &protected(),
                    "sha256:payload",
                    "sha256:moved",
                ),
            ),
            (
                "protection_unverified",
                verify(
                    &manifest(),
                    &authority(),
                    &base(vec![approve(200, HEAD)]),
                    &ProtectionEvidence::default(),
                    "sha256:payload",
                    "sha256:base",
                ),
            ),
        ];
        for (code, result) in cases {
            assert_eq!(result.expect_err(code).code, code);
        }
        let mut expired = manifest();
        expired.expires_at = "2026-01-01T00:00:00Z".into();
        assert_eq!(
            verify(
                &expired,
                &authority(),
                &merge(vec![approve(200, HEAD)]),
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("expired")
            .code,
            "expired"
        );
        let mut stale_authority = manifest();
        stale_authority.authority_revision = 2;
        assert_eq!(
            verify(
                &stale_authority,
                &authority(),
                &merge(vec![approve(200, HEAD)]),
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("authority")
            .code,
            "authority_stale"
        );
        let mut revoked = authority();
        revoked.revoked = vec![200];
        assert_eq!(
            verify(
                &manifest(),
                &revoked,
                &merge(vec![approve(200, HEAD)]),
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("revoked")
            .code,
            "no_independent_approval"
        );
        let mut scoped = manifest();
        scoped.scope = vec!["checkout".into()];
        assert_eq!(
            verify(
                &scoped,
                &authority(),
                &merge(vec![approve(300, HEAD)]),
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("scope")
            .code,
            "no_independent_approval"
        );
        let mut unmerged = merge(vec![approve(200, HEAD)]);
        unmerged.merged = false;
        assert_eq!(
            verify(
                &manifest(),
                &authority(),
                &unmerged,
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("merge")
            .code,
            "not_merged"
        );
        let mut elsewhere = merge(vec![approve(200, HEAD)]);
        elsewhere.repository_id = 43;
        assert_eq!(
            verify(
                &manifest(),
                &authority(),
                &elsewhere,
                &protected(),
                "sha256:payload",
                "sha256:base"
            )
            .expect_err("repo")
            .code,
            "repository_mismatch"
        );
    }

    #[test]
    fn a_forged_approval_field_invalidates_the_manifest() {
        let mut value = serde_json::to_value(manifest()).expect("json");
        value["approved_by"] = serde_json::json!("grace");
        assert!(serde_json::from_value::<ActivationManifest>(value).is_err());
    }
}
