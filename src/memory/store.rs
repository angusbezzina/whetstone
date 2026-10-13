//! One lesson store interface, two backends. The contract (who may admit,
//! scope, anchors, staleness, the ladder) is decided before a store is
//! called; a store only keeps lessons and performs the four verbs.

use std::collections::BTreeMap;

use crate::domain::{
    ActorKind, AgreementRecord, EvidenceRef, ForgetProposal, Lesson, LessonDecision, LessonStamp,
    LessonStatus, LessonVerb, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordRef, Trust,
};
use crate::storage::{ProjectLayout, RecordStore, StoreKind};

use super::{MemoryConfig, Refusal};

/// A lesson as a store holds it: its latest revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLesson {
    pub lesson: Lesson,
    pub revision: u64,
    /// The local record revision (Beads backend).
    pub reference: Option<RecordRef>,
    /// Where it lives: `whetstone:lesson.<key>@<revision>` or `virgil:<id>`.
    pub locator: String,
}

/// The four verbs plus the two reads, over either backend. Every write takes
/// an idempotency key so a repeated request returns what it recorded.
pub trait LessonStore {
    fn backend(&self) -> &'static str;

    /// The latest revision of every lesson, by key order.
    fn find(&self) -> Result<Vec<StoredLesson>, Refusal>;

    /// Lessons waiting on a person: proposed lessons and proposed forgets.
    fn proposals(&self) -> Result<Vec<StoredLesson>, Refusal> {
        Ok(self
            .find()?
            .into_iter()
            .filter(|stored| {
                stored.lesson.status == LessonStatus::Proposed
                    || (stored.lesson.forget_proposed.is_some()
                        && stored.lesson.status == LessonStatus::Admitted)
            })
            .collect())
    }

    fn get(&self, key: &str) -> Result<Option<StoredLesson>, Refusal> {
        Ok(self
            .find()?
            .into_iter()
            .find(|stored| stored.lesson.key == key))
    }

    /// Propose a lesson (or, with `lesson.supersedes`, a correction).
    fn remember(&self, lesson: &Lesson, idempotency: &str) -> Result<StoredLesson, Refusal>;

    /// A person admits a proposed lesson; admitting a correction supersedes
    /// the lesson it corrects.
    fn admit(
        &self,
        key: &str,
        trust: Trust,
        decision: &LessonDecision,
        stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal>;

    /// Propose a correction to `key` (the lesson's `supersedes` names it).
    fn supersede(
        &self,
        key: &str,
        correction: &Lesson,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal>;

    /// A person forgets a lesson (`decision`), or anyone proposes forgetting
    /// it (`proposal`).
    fn forget(
        &self,
        key: &str,
        decision: Option<&LessonDecision>,
        proposal: Option<&ForgetProposal>,
        stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal>;
}

/// Open the store the switch names.
pub fn open_store(layout: &ProjectLayout, config: &MemoryConfig) -> Box<dyn LessonStore> {
    match config {
        MemoryConfig::Beads => Box::new(BeadsLessons {
            layout: layout.clone(),
        }),
        MemoryConfig::Virgil { group } => Box::new(super::virgil::VirgilLessons::new(
            layout.project_root(),
            group,
        )),
    }
}

/// Standalone: lessons are `lesson` records in Whetstone's private Beads
/// store, one revision per verb, append-only.
pub struct BeadsLessons {
    pub layout: ProjectLayout,
}

fn storage_refusal(error: impl std::fmt::Display) -> Refusal {
    Refusal::new(
        "store_unavailable",
        format!("The lesson store failed: {error}"),
    )
}

impl BeadsLessons {
    /// Every lesson revision in the private and shared stores.
    pub fn revisions(&self) -> Result<Vec<AgreementRecord>, Refusal> {
        let loaded = crate::service::load_records(&self.layout).map_err(storage_refusal)?;
        Ok(loaded
            .union()
            .0
            .into_iter()
            .filter(|record| matches!(record.body, RecordBody::Lesson(_)))
            .collect())
    }

    fn store(&self) -> Result<RecordStore, Refusal> {
        RecordStore::initialize(&self.layout.private_store(), StoreKind::Private)
            .map_err(storage_refusal)
    }

    fn latest_record(&self, key: &str) -> Result<Option<AgreementRecord>, Refusal> {
        let id = super::record_id(key).map_err(|error| {
            Refusal::new(
                "invalid_lesson",
                format!("{key} is not a lesson key: {error}"),
            )
        })?;
        Ok(self
            .revisions()?
            .into_iter()
            .filter(|record| record.id == id)
            .max_by_key(|record| record.revision))
    }

    fn stored(record: &AgreementRecord) -> Option<StoredLesson> {
        let RecordBody::Lesson(lesson) = &record.body else {
            return None;
        };
        Some(StoredLesson {
            lesson: lesson.clone(),
            revision: record.revision,
            reference: record.reference().ok(),
            locator: format!("whetstone:{}@{}", record.id.as_str(), record.revision),
        })
    }

    /// Append the next revision of a lesson, or return what this key recorded.
    fn append(&self, lesson: Lesson, idempotency: &str) -> Result<StoredLesson, Refusal> {
        let store = self.store()?;
        if let Some(existing) = store
            .by_idempotency_key(idempotency)
            .map_err(storage_refusal)?
        {
            return Self::stored(&existing).ok_or_else(|| {
                Refusal::new(
                    "idempotency_conflict",
                    "That request id recorded something other than a lesson.",
                )
            });
        }
        lesson.validate().map_err(|error| {
            Refusal::new(
                "invalid_lesson",
                format!("The lesson is not valid: {error}"),
            )
        })?;
        let id = super::record_id(&lesson.key)
            .map_err(|error| Refusal::new("invalid_lesson", error.to_string()))?;
        let current = self.latest_record(&lesson.key)?;
        let person = lesson.stamp.actor == ActorKind::Person
            && lesson
                .decision
                .as_ref()
                .is_some_and(|decision| decision.verb != LessonVerb::Remember);
        let mut record = crate::service::agreement_record(
            &self.layout,
            id.as_str(),
            idempotency.to_string(),
            RecordBody::Lesson(lesson.clone()),
            Some(&lesson.stamp.who),
        )
        .map_err(|error| Refusal::new("invalid_lesson", error.to_string()))?;
        record.revision = current.as_ref().map_or(1, |current| current.revision + 1);
        record.supersedes = current
            .as_ref()
            .map(AgreementRecord::reference)
            .transpose()
            .map_err(|error| Refusal::new("invalid_lesson", error.to_string()))?;
        record.provenance.recorded_at = lesson.stamp.at.clone();
        record.provenance.sources = vec![EvidenceRef {
            system: "whetstone_lesson".into(),
            locator: format!("{} via wh change", lesson.stamp.actor.label()),
            digest: None,
        }];
        // A person's decision is owner-authored; anything an agent wrote is
        // a candidate only.
        record.provenance.kind = match lesson.stamp.actor {
            ActorKind::Person => ProvenanceKind::HumanAuthored,
            ActorKind::Agent => ProvenanceKind::InferredPreference,
        };
        record.provenance.authority = if person {
            ProvenanceAuthority::OwnerAuthored
        } else {
            ProvenanceAuthority::CandidateOnly
        };
        let expected = current.as_ref().map(|current| current.revision);
        store.append(&record, expected).map_err(storage_refusal)?;
        Self::stored(&record).ok_or_else(|| storage_refusal("the lesson did not round-trip"))
    }

    fn require(&self, key: &str) -> Result<Lesson, Refusal> {
        match self.latest_record(key)?.map(|record| record.body) {
            Some(RecordBody::Lesson(lesson)) => Ok(lesson),
            _ => Err(Refusal::new(
                "lesson_not_found",
                format!("No lesson {key} exists; see wh dash --json (current.lessons)."),
            )),
        }
    }
}

impl LessonStore for BeadsLessons {
    fn backend(&self) -> &'static str {
        "beads"
    }

    fn find(&self) -> Result<Vec<StoredLesson>, Refusal> {
        let mut latest = BTreeMap::<String, AgreementRecord>::new();
        for record in self.revisions()? {
            let RecordBody::Lesson(lesson) = &record.body else {
                continue;
            };
            let newer = latest
                .get(&lesson.key)
                .map_or(true, |current| current.revision < record.revision);
            if newer {
                latest.insert(lesson.key.clone(), record);
            }
        }
        Ok(latest.values().filter_map(Self::stored).collect())
    }

    fn remember(&self, lesson: &Lesson, idempotency: &str) -> Result<StoredLesson, Refusal> {
        if let Some(existing) = self.latest_record(&lesson.key)? {
            let store = self.store()?;
            let replay = store
                .by_idempotency_key(idempotency)
                .map_err(storage_refusal)?;
            if replay.is_none() {
                return Err(Refusal::new(
                    "lesson_exists",
                    format!(
                        "A lesson {} already exists (revision {}); correct it with wh change --supersede {} instead.",
                        lesson.key, existing.revision, lesson.key
                    ),
                ));
            }
        }
        self.append(lesson.clone(), idempotency)
    }

    fn admit(
        &self,
        key: &str,
        trust: Trust,
        decision: &LessonDecision,
        stamp: &LessonStamp,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        let current = self.require(key)?;
        if current.status != LessonStatus::Proposed {
            if let Some(replay) = self
                .store()?
                .by_idempotency_key(idempotency)
                .map_err(storage_refusal)?
            {
                return Self::stored(&replay).ok_or_else(|| storage_refusal("not a lesson"));
            }
            return Err(Refusal::new(
                "lesson_not_proposed",
                format!(
                    "{key} is {}, not proposed; nothing to admit.",
                    current.status.label()
                ),
            ));
        }
        let mut next = super::decided(&current, decision.clone(), stamp.clone());
        next.trust = trust;
        let admitted = self.append(next, idempotency)?;
        // Admitting a correction supersedes the lesson it corrects.
        if let Some(old_key) = current.supersedes.as_deref() {
            if let Ok(old) = self.require(old_key) {
                if !matches!(
                    old.status,
                    LessonStatus::Superseded | LessonStatus::Forgotten
                ) {
                    let mut superseded = super::decided(
                        &old,
                        LessonDecision {
                            verb: LessonVerb::Supersede,
                            ..decision.clone()
                        },
                        stamp.clone(),
                    );
                    superseded.superseded_by = Some(key.to_string());
                    self.append(superseded, &format!("{idempotency}:supersedes:{old_key}"))?;
                }
            }
        }
        Ok(admitted)
    }

    fn supersede(
        &self,
        key: &str,
        correction: &Lesson,
        idempotency: &str,
    ) -> Result<StoredLesson, Refusal> {
        let old = self.require(key)?;
        if matches!(
            old.status,
            LessonStatus::Superseded | LessonStatus::Forgotten
        ) {
            return Err(Refusal::new(
                "lesson_not_live",
                format!(
                    "{key} is {}; correct the lesson that replaced it.",
                    old.status.label()
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
        let current = self.require(key)?;
        if current.status == LessonStatus::Forgotten {
            if let Some(replay) = self
                .store()?
                .by_idempotency_key(idempotency)
                .map_err(storage_refusal)?
            {
                return Self::stored(&replay).ok_or_else(|| storage_refusal("not a lesson"));
            }
            return Err(Refusal::new(
                "lesson_not_live",
                format!("{key} is already forgotten."),
            ));
        }
        let next = match (decision, proposal) {
            (Some(decision), _) => super::decided(&current, decision.clone(), stamp.clone()),
            (None, Some(proposal)) => {
                super::with_forget_proposal(&current, proposal.clone(), stamp.clone())
            }
            (None, None) => {
                return Err(Refusal::new(
                    "invalid_request",
                    "Forget needs a decision or a proposal.",
                ))
            }
        };
        self.append(next, idempotency)
    }
}
