//! SQLite persistence for courier crash recovery.
//!
//! Stores `EthExecutionResponse` entries and `PendingBatch` state so the
//! courier can resume without re-requesting already-computed witnesses after
//! a crash.
//!
//! Schema:
//! - `block_responses(block_number PK, response BLOB)` — serialized responses
//! - `pending_batches(batch_index PK, from_block, to_block, blobs_accepted)`
//! - `pending_blobs_accepted(batch_index PK)` — buffered pre-registration events
//! - `meta(key PK, value)` — checkpoint and other scalars
//!
//! All writes are immediately durable (WAL mode, synchronous=NORMAL).

use std::path::Path;

use rusqlite::{params, Connection, Result};
use tracing::error;

use crate::accumulator::PendingBatch;
use crate::types::{EthExecutionResponse, SubmitBatchResponse};

pub struct Db {
    conn: Connection,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

impl Db {
    /// Open or create the SQLite database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS block_responses (
                block_number INTEGER PRIMARY KEY,
                response     BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pending_batches (
                batch_index   INTEGER PRIMARY KEY,
                from_block    INTEGER NOT NULL,
                to_block      INTEGER NOT NULL,
                blobs_accepted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS pending_blobs_accepted (
                batch_index INTEGER PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS batch_signatures (
                batch_index INTEGER PRIMARY KEY,
                response    BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ")?;
        Ok(Self { conn })
    }

    // ── Checkpoint ──────────────────────────────────────────────────────────

    pub fn get_checkpoint(&self) -> u64 {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'checkpoint'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn save_checkpoint(&self, block_number: u64) {
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('checkpoint', ?1)",
            params![block_number.to_string()],
        ) {
            error!(err = %e, "Failed to persist checkpoint");
        }
    }

    // ── L1 checkpoint ────────────────────────────────────────────────────────────

    /// Last L1 block successfully polled by the L1 listener.
    /// On restart, the listener resumes from `get_l1_checkpoint().map(|b| b + 1).unwrap_or(l1_start_block)`.
    /// Returns `None` if never set — distinguishes "not persisted" from "persisted block 0".
    pub fn get_l1_checkpoint(&self) -> Option<u64> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'l1_checkpoint'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| s.parse().ok())
    }

    pub fn save_l1_checkpoint(&self, block_number: u64) {
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('l1_checkpoint', ?1)",
            params![block_number.to_string()],
        ) {
            error!(err = %e, "Failed to persist l1_checkpoint");
        }
    }

    // ── Last batch end ────────────────────────────────────────────────────────────

    /// to_block of the last successfully preconfirmed batch.
    /// Used to recover `next_batch_from_block` on restart when pending_batches is empty.
    /// Returns None if no batch has ever been preconfirmed.
    pub fn get_last_batch_end(&self) -> Option<u64> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_batch_end'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| s.parse().ok())
    }

    pub fn save_last_batch_end(&self, block_number: u64) {
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('last_batch_end', ?1)",
            params![block_number.to_string()],
        ) {
            error!(err = %e, "Failed to persist last_batch_end");
        }
    }

    // ── Responses ───────────────────────────────────────────────────────────

    pub fn save_response(&self, resp: &EthExecutionResponse) {
        let blob = match bincode::serialize(resp) {
            Ok(b) => b,
            Err(e) => {
                error!(err = %e, "Failed to serialize EthExecutionResponse");
                return;
            }
        };
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO block_responses(block_number, response) VALUES(?1, ?2)",
            params![resp.block_number, blob],
        ) {
            error!(err = %e, block_number = resp.block_number, "Failed to persist block response");
        }
    }

    pub fn delete_responses(&self, from_block: u64, to_block: u64) {
        if let Err(e) = self.conn.execute(
            "DELETE FROM block_responses WHERE block_number BETWEEN ?1 AND ?2",
            params![from_block, to_block],
        ) {
            error!(err = %e, "Failed to delete block responses");
        }
    }

    pub fn delete_response(&self, block_number: u64) {
        if let Err(e) = self.conn.execute(
            "DELETE FROM block_responses WHERE block_number = ?1",
            params![block_number],
        ) {
            error!(err = %e, block_number, "Failed to delete block response");
        }
    }

    pub fn load_responses(&self) -> Vec<EthExecutionResponse> {
        let mut stmt = match self.conn.prepare(
            "SELECT response FROM block_responses ORDER BY block_number",
        ) {
            Ok(s) => s,
            Err(e) => {
                error!(err = %e, "load_responses prepare failed");
                return vec![];
            }
        };
        let blobs: Vec<Vec<u8>> = stmt
            .query_map([], |row| row.get(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        blobs
            .into_iter()
            .filter_map(|b| bincode::deserialize(&b).ok())
            .collect()
    }

    pub fn get_all_response_block_numbers(&self) -> Vec<u64> {
        let mut stmt = match self.conn.prepare(
            "SELECT block_number FROM block_responses ORDER BY block_number",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).map(|n| n as u64).collect())
            .unwrap_or_default()
    }

    // ── Batches ─────────────────────────────────────────────────────────────

    pub fn save_batch(&self, batch: &PendingBatch) {
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO pending_batches(batch_index, from_block, to_block, blobs_accepted)
             VALUES(?1, ?2, ?3, ?4)",
            params![batch.batch_index, batch.from_block, batch.to_block, batch.blobs_accepted as i64],
        ) {
            error!(err = %e, batch_index = batch.batch_index, "Failed to persist batch");
        }
    }

    pub fn update_blobs_accepted(&self, batch_index: u64) {
        if let Err(e) = self.conn.execute(
            "UPDATE pending_batches SET blobs_accepted = 1 WHERE batch_index = ?1",
            params![batch_index],
        ) {
            error!(err = %e, batch_index, "Failed to update blobs_accepted");
        }
    }

    pub fn delete_batch(&self, batch_index: u64) {
        if let Err(e) = self.conn.execute(
            "DELETE FROM pending_batches WHERE batch_index = ?1",
            params![batch_index],
        ) {
            error!(err = %e, batch_index, "Failed to delete batch");
        }
    }

    pub fn load_batches(&self) -> Vec<PendingBatch> {
        let mut stmt = match self.conn.prepare(
            "SELECT batch_index, from_block, to_block, blobs_accepted FROM pending_batches ORDER BY batch_index",
        ) {
            Ok(s) => s,
            Err(e) => {
                error!(err = %e, "Failed to prepare load_batches");
                return vec![];
            }
        };
        stmt.query_map([], |row| {
            Ok(PendingBatch {
                batch_index: row.get::<_, i64>(0)? as u64,
                from_block:  row.get::<_, i64>(1)? as u64,
                to_block:    row.get::<_, i64>(2)? as u64,
                blobs_accepted: row.get::<_, i64>(3)? != 0,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    // ── Batch signatures ─────────────────────────────────────────────────────

    pub fn save_batch_signature(&self, batch_index: u64, resp: &SubmitBatchResponse) {
        let blob = match bincode::serialize(resp) {
            Ok(b) => b,
            Err(e) => {
                error!(err = %e, batch_index, "Failed to serialize SubmitBatchResponse");
                return;
            }
        };
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO batch_signatures(batch_index, response) VALUES(?1, ?2)",
            params![batch_index, blob],
        ) {
            error!(err = %e, batch_index, "Failed to persist batch signature");
        }
    }

    pub fn get_batch_signature(&self, batch_index: u64) -> Option<SubmitBatchResponse> {
        let blob: Vec<u8> = self
            .conn
            .query_row(
                "SELECT response FROM batch_signatures WHERE batch_index = ?1",
                params![batch_index],
                |row| row.get(0),
            )
            .ok()?;
        bincode::deserialize(&blob).ok()
    }

    pub fn has_batch_signature(&self, batch_index: u64) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM batch_signatures WHERE batch_index = ?1",
                params![batch_index],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn delete_batch_signature(&self, batch_index: u64) {
        if let Err(e) = self.conn.execute(
            "DELETE FROM batch_signatures WHERE batch_index = ?1",
            params![batch_index],
        ) {
            error!(err = %e, batch_index, "Failed to delete batch signature");
        }
    }

    // ── Pending blobs accepted ───────────────────────────────────────────────

    pub fn save_pending_blobs_accepted(&self, batch_index: u64) {
        if let Err(e) = self.conn.execute(
            "INSERT OR IGNORE INTO pending_blobs_accepted(batch_index) VALUES(?1)",
            params![batch_index],
        ) {
            error!(err = %e, batch_index, "Failed to save pending_blobs_accepted");
        }
    }

    pub fn delete_pending_blobs_accepted(&self, batch_index: u64) {
        if let Err(e) = self.conn.execute(
            "DELETE FROM pending_blobs_accepted WHERE batch_index = ?1",
            params![batch_index],
        ) {
            error!(err = %e, batch_index, "Failed to delete pending_blobs_accepted");
        }
    }

    pub fn load_pending_blobs_accepted(&self) -> Vec<u64> {
        let mut stmt = match self.conn.prepare(
            "SELECT batch_index FROM pending_blobs_accepted",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).map(|n| n as u64).collect())
            .unwrap_or_default()
    }
}
