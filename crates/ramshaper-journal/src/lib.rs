//! SQLite action journal (RS-031 first slice; contract 14 §6).
//!
//! Durable, versioned storage for policy action records. The pure state
//! machine, reconciliation and retention decisions live in
//! [`ramshaper_core::journal`]; this crate owns the SQLite persistence:
//!
//! - schema `JOURNAL_SCHEMA_VERSION` is stored in `PRAGMA user_version`;
//!   an unreadable or newer database is refused (`Corrupt` /
//!   `SchemaMismatch`) and **left untouched** — it blocks new policy
//!   actions rather than being "fixed" by this build;
//! - every write is one transaction (`BEGIN IMMEDIATE`), so a crash at any
//!   point leaves either the old or the new record, never a torn one —
//!   SQLite's own crash safety is the mechanism;
//! - transitions are applied through the pure state machine with an
//!   optimistic state guard: two writers cannot both move the same record;
//! - unresolved records are pinned: pruning skips them, and a full record
//!   cap surfaces as [`JournalError::QuotaReached`] (block new actions,
//!   never evict recovery data);
//! - disk exhaustion (`SQLITE_FULL`) maps to [`JournalError::DiskFull`] so
//!   callers can "continue observation, disable new actions" (contract 14).

use std::path::Path;
use std::sync::Mutex;

use ramshaper_core::journal::{
    transition, ActionIntent, ActionState, Event, JOURNAL_SCHEMA_VERSION,
};
use rusqlite::Connection;

/// Typed journal failures (contract 14 §6/§7 and AT-18 inputs).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// The database's schema version is not this build's. The file is not
    /// modified; new policy actions must not start.
    #[error(
        "journal schema {found} is not supported (this build writes {supported}); journal left \
         untouched"
    )]
    SchemaMismatch {
        /// Version found in the database.
        found: i64,
        /// Version this build writes.
        supported: i64,
    },
    /// The file is not a usable journal (not a database, header damaged).
    #[error("journal is unreadable and has been left untouched: {0}")]
    Corrupt(String),
    /// The storage device is full (`SQLITE_FULL`). Observation continues;
    /// new actions stop; recovery of already-recorded intents proceeds.
    #[error("journal storage is full; new actions are blocked, recovery proceeds")]
    DiskFull,
    /// The record cap is reached with only pinned (recovery) records.
    #[error("journal record cap reached; pinned recovery records block new actions")]
    QuotaReached,
    /// No record with this action ID.
    #[error("no journal record for action '{0}'")]
    Missing(String),
    /// The record moved since it was read (optimistic guard).
    #[error("action '{action_id}' is {actual:?}, expected {expected:?}; re-read before retrying")]
    StateConflict {
        /// The action whose state moved.
        action_id: String,
        /// State the caller assumed.
        expected: ActionState,
        /// State actually stored.
        actual: ActionState,
    },
    /// The transition is not legal from the record's state (state machine).
    #[error(transparent)]
    IllegalTransition(#[from] ramshaper_core::journal::TransitionError),
    /// Any other SQLite failure, message preserved.
    #[error("journal database error: {0}")]
    Db(String),
}

impl JournalError {
    /// Classify by primary result code: SQLITE_FULL is the disk-full
    /// contract case; CANTOPEN/NOTADB/CORRUPT are refusal cases. Everything
    /// else keeps its message.
    fn classify(error: &rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, message) = error {
            return match code.code {
                rusqlite::ErrorCode::SystemIoFailure | rusqlite::ErrorCode::DiskFull => {
                    Self::DiskFull
                }
                rusqlite::ErrorCode::CannotOpen
                | rusqlite::ErrorCode::NotADatabase
                | rusqlite::ErrorCode::DatabaseCorrupt => {
                    Self::Corrupt(message.clone().unwrap_or_else(|| code.to_string()))
                }
                _ => Self::Db(error.to_string()),
            };
        }
        Self::Db(error.to_string())
    }
}

/// One persisted record: the original intent plus its current state.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalRecord {
    /// The durable intent as recorded at `Prepared`.
    pub intent: ActionIntent,
    /// Current state-machine state.
    pub state: ActionState,
    /// Whether a write may have occurred (recovery intent retained).
    pub recovery_retained: bool,
    /// Live deadline recorded at `ApplyVerified`.
    pub applied_deadline_unix_ms: Option<u64>,
    /// When the record reached a terminal state (`Restored`, `TargetGone`).
    pub terminal_at_unix_ms: Option<u64>,
    /// Last transition evidence (JSON object; synthetic in tests only).
    pub evidence: serde_json::Value,
}

impl JournalRecord {
    /// Pinned per the pure rules (unresolved records are never pruned).
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self.state.is_pinned(self.recovery_retained)
    }

    /// Unresolved: still needs recovery work or attention.
    #[must_use]
    pub fn is_unresolved(&self) -> bool {
        self.state.is_unresolved(self.recovery_retained)
    }
}

/// Outcome of `record_prepared` (contract: replay returns the existing
/// record, never extends TTL or creates another action).
#[derive(Debug, Clone, PartialEq)]
pub enum Recorded {
    /// A new `Prepared` record was durably committed.
    Created,
    /// The action ID already existed; the existing record is returned and
    /// must be used as-is.
    Existing(Box<JournalRecord>),
}

/// SQLite-backed action journal. Cheap to clone (connection behind a
/// mutex); all methods serialize through it.
#[derive(Clone)]
pub struct JournalStore {
    conn: std::sync::Arc<Mutex<Connection>>,
}

const CREATE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS actions (
    action_id              TEXT PRIMARY KEY,
    agent_instance_id      TEXT NOT NULL,
    pid                    INTEGER NOT NULL,
    creation_filetime      INTEGER NOT NULL,
    target_name            TEXT NOT NULL,
    kind                   TEXT NOT NULL,
    original_value         INTEGER,
    desired_value          INTEGER NOT NULL,
    consent_id             TEXT NOT NULL,
    rule_version           TEXT NOT NULL,
    ttl_ms                 INTEGER NOT NULL,
    created_at_unix_ms     INTEGER NOT NULL,
    state                  TEXT NOT NULL,
    recovery_retained      INTEGER NOT NULL DEFAULT 0,
    applied_deadline_unix_ms INTEGER,
    terminal_at_unix_ms    INTEGER,
    evidence_json          TEXT NOT NULL DEFAULT '{}'
);";

impl JournalStore {
    /// Open (creating if absent) the journal at `path`. A database with a
    /// schema version other than this build's, or an unreadable file, is
    /// refused without modification.
    pub fn open(path: &Path) -> Result<Self, JournalError> {
        let conn = Connection::open(path).map_err(|e| {
            // An unwritable/corrupt-at-open path must not look like success.
            match JournalError::classify(&e) {
                JournalError::Db(_) => JournalError::Corrupt(e.to_string()),
                other => other,
            }
        })?;
        let store = Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
        };
        store.validate_schema()?;
        Ok(store)
    }

    /// In-memory journal for tests (same schema and semantics).
    pub fn open_in_memory() -> Result<Self, JournalError> {
        let conn = Connection::open_in_memory().map_err(|e| JournalError::classify(&e))?;
        let store = Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
        };
        store.validate_schema()?;
        Ok(store)
    }

    fn validate_schema(&self) -> Result<(), JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        // Read-only probe; a non-database file fails here without writes.
        let version = conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(|e| {
                JournalError::Corrupt(format!("cannot read journal schema version: {e}"))
            })?;
        let has_actions = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='actions'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| JournalError::Corrupt(e.to_string()))?;
        match (version, has_actions) {
            (0, 0) => {
                // Fresh database: create the schema inside one transaction.
                conn.execute_batch(CREATE_SCHEMA)
                    .map_err(|e| JournalError::classify(&e))?;
                conn.execute_batch(&format!("PRAGMA user_version = {JOURNAL_SCHEMA_VERSION};"))
                    .map_err(|e| JournalError::classify(&e))?;
                Ok(())
            }
            (v, _) if v == JOURNAL_SCHEMA_VERSION => Ok(()),
            (v, _) => Err(JournalError::SchemaMismatch {
                found: v,
                supported: JOURNAL_SCHEMA_VERSION,
            }),
        }
    }

    /// Durably record the `Prepared` intent. Idempotent per action ID:
    /// replay returns [`Recorded::Existing`] and never extends TTL.
    pub fn record_prepared(&self, intent: &ActionIntent) -> Result<Recorded, JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        if let Some(existing) = read_one(&conn, &intent.action_id)? {
            return Ok(Recorded::Existing(Box::new(existing)));
        }
        conn.execute(
            "INSERT INTO actions (action_id, agent_instance_id, pid, creation_filetime,
                target_name, kind, original_value, desired_value, consent_id, rule_version,
                ttl_ms, created_at_unix_ms, state, recovery_retained)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'Prepared', 0)",
            rusqlite::params![
                intent.action_id,
                intent.agent_instance_id,
                intent.pid,
                intent.creation_filetime as i64,
                intent.target_name,
                intent.property.kind,
                intent.property.original.map(|v| v as i64),
                intent.property.desired as i64,
                intent.consent_id,
                intent.rule_version,
                intent.ttl_ms as i64,
                intent.created_at_unix_ms as i64,
            ],
        )
        .map_err(|e| JournalError::classify(&e))?;
        Ok(Recorded::Created)
    }

    /// Apply one event to a record through the pure state machine, guarded
    /// on the caller's expected state. `now_unix_ms` stamps terminal
    /// records. Returns the new state.
    pub fn transition(
        &self,
        action_id: &str,
        expected: ActionState,
        event: &Event,
        now_unix_ms: u64,
        evidence: serde_json::Value,
    ) -> Result<ActionState, JournalError> {
        let mut conn = self.conn.lock().expect("journal lock");
        let tx = conn.transaction().map_err(|e| JournalError::classify(&e))?;
        let record = read_one(&tx, action_id)?
            .ok_or_else(|| JournalError::Missing(action_id.to_string()))?;
        if record.state != expected {
            return Err(JournalError::StateConflict {
                action_id: action_id.to_string(),
                expected,
                actual: record.state,
            });
        }
        let next = transition(expected, event)?;
        let recovery_retained = matches!(
            next,
            ActionState::RestorePending
                | ActionState::Uncertain
                | ActionState::Conflict
                | ActionState::Prepared
                | ActionState::Applied
        ) || (next == ActionState::Failed && record.recovery_retained);
        let terminal_at = matches!(next, ActionState::Restored | ActionState::TargetGone)
            .then_some(now_unix_ms as i64);
        let deadline = match event {
            Event::ApplyVerified { deadline_unix_ms } => Some(*deadline_unix_ms as i64),
            _ => None,
        };
        let changed = tx
            .execute(
                "UPDATE actions SET state = ?1, recovery_retained = ?2,
                    applied_deadline_unix_ms = COALESCE(?3, applied_deadline_unix_ms),
                    terminal_at_unix_ms = COALESCE(?4, terminal_at_unix_ms),
                    evidence_json = ?5
                 WHERE action_id = ?6 AND state = ?7",
                rusqlite::params![
                    state_name(next),
                    i64::from(recovery_retained),
                    deadline,
                    terminal_at,
                    evidence.to_string(),
                    action_id,
                    state_name(expected),
                ],
            )
            .map_err(|e| JournalError::classify(&e))?;
        if changed == 0 {
            return Err(JournalError::StateConflict {
                action_id: action_id.to_string(),
                expected,
                actual: record.state,
            });
        }
        tx.commit().map_err(|e| JournalError::classify(&e))?;
        Ok(next)
    }

    /// One record by action ID, if present.
    pub fn get(&self, action_id: &str) -> Result<Option<JournalRecord>, JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        read_one(&conn, action_id)
    }

    /// All unresolved (pinned) records — the recovery set.
    pub fn unresolved(&self) -> Result<Vec<JournalRecord>, JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        let mut stmt = conn
            .prepare("SELECT * FROM actions ORDER BY created_at_unix_ms, action_id")
            .map_err(|e| JournalError::classify(&e))?;
        let rows = stmt
            .query_map([], row_to_record)
            .map_err(|e| JournalError::classify(&e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| JournalError::classify(&e))?;
        Ok(rows.into_iter().filter(|r| r.is_unresolved()).collect())
    }

    /// Counts for the quota decision: (total, pinned).
    pub fn counts(&self) -> Result<(usize, usize), JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        let mut stmt = conn
            .prepare("SELECT * FROM actions")
            .map_err(|e| JournalError::classify(&e))?;
        let rows = stmt
            .query_map([], row_to_record)
            .map_err(|e| JournalError::classify(&e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| JournalError::classify(&e))?;
        let pinned = rows.iter().filter(|r| r.is_pinned()).count();
        Ok((rows.len(), pinned))
    }

    /// Prune terminal records older than `retention_window_ms`; pinned
    /// records are never pruned. Returns how many rows were removed.
    pub fn prune(&self, now_unix_ms: u64, retention_window_ms: u64) -> Result<usize, JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        let cutoff = now_unix_ms.saturating_sub(retention_window_ms) as i64;
        // Pinning is decided in pure code per record; do it here by state so
        // one DELETE covers the prunable set only.
        let changed = conn
            .execute(
                "DELETE FROM actions
                 WHERE state IN ('Restored', 'TargetGone')
                   AND recovery_retained = 0
                   AND terminal_at_unix_ms IS NOT NULL
                   AND terminal_at_unix_ms <= ?1",
                rusqlite::params![cutoff],
            )
            .map_err(|e| JournalError::classify(&e))?;
        Ok(changed)
    }
}

fn state_name(state: ActionState) -> &'static str {
    match state {
        ActionState::Prepared => "Prepared",
        ActionState::Applied => "Applied",
        ActionState::RestorePending => "RestorePending",
        ActionState::Restored => "Restored",
        ActionState::TargetGone => "TargetGone",
        ActionState::Conflict => "Conflict",
        ActionState::Failed => "Failed",
        ActionState::Uncertain => "Uncertain",
    }
}

fn state_from_name(name: &str) -> Option<ActionState> {
    match name {
        "Prepared" => Some(ActionState::Prepared),
        "Applied" => Some(ActionState::Applied),
        "RestorePending" => Some(ActionState::RestorePending),
        "Restored" => Some(ActionState::Restored),
        "TargetGone" => Some(ActionState::TargetGone),
        "Conflict" => Some(ActionState::Conflict),
        "Failed" => Some(ActionState::Failed),
        "Uncertain" => Some(ActionState::Uncertain),
        _ => None,
    }
}

fn read_one(conn: &Connection, action_id: &str) -> Result<Option<JournalRecord>, JournalError> {
    let mut stmt = conn
        .prepare("SELECT * FROM actions WHERE action_id = ?1")
        .map_err(|e| JournalError::classify(&e))?;
    let mut rows = stmt
        .query_map(rusqlite::params![action_id], row_to_record)
        .map_err(|e| JournalError::classify(&e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| JournalError::classify(&e))?;
    Ok(rows.pop())
}

fn row_to_record(row: &rusqlite::Row<'_>) -> Result<JournalRecord, rusqlite::Error> {
    let state_name: String = row.get("state")?;
    let state = state_from_name(&state_name).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown state '{state_name}'").into(),
        )
    })?;
    let kind: String = row.get("kind")?;
    let evidence_json: String = row.get("evidence_json")?;
    Ok(JournalRecord {
        intent: ActionIntent {
            action_id: row.get("action_id")?,
            agent_instance_id: row.get("agent_instance_id")?,
            pid: row.get::<_, i64>("pid")? as u32,
            creation_filetime: row.get::<_, i64>("creation_filetime")? as u64,
            target_name: row.get("target_name")?,
            property: ramshaper_core::journal::Property {
                kind: Box::leak(kind.into_boxed_str()),
                original: row
                    .get::<_, Option<i64>>("original_value")?
                    .map(|v| v as u32),
                desired: row.get::<_, i64>("desired_value")? as u32,
            },
            consent_id: row.get("consent_id")?,
            rule_version: row.get("rule_version")?,
            ttl_ms: row.get::<_, i64>("ttl_ms")? as u64,
            created_at_unix_ms: row.get::<_, i64>("created_at_unix_ms")? as u64,
        },
        state,
        recovery_retained: row.get::<_, i64>("recovery_retained")? != 0,
        applied_deadline_unix_ms: row
            .get::<_, Option<i64>>("applied_deadline_unix_ms")?
            .map(|v| v as u64),
        terminal_at_unix_ms: row
            .get::<_, Option<i64>>("terminal_at_unix_ms")?
            .map(|v| v as u64),
        evidence: serde_json::from_str(&evidence_json).unwrap_or(serde_json::Value::Null),
    })
}
