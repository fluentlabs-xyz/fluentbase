use crate::{Bytes, B256, U256};
use alloc::vec::Vec;
use bincode::{
    de::Decoder,
    enc::Encoder,
    error::{DecodeError, EncodeError},
};
use hashbrown::HashMap;

/// A single EVM-style log entry recorded during execution.
///
/// This mirrors Ethereum LOG op semantics:
/// - `topics` correspond to indexed event topics
/// - `data` is the opaque event payload
///
/// Logs are accumulated during execution and emitted on commit.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct JournalLog {
    /// Indexed log topics (0–4 in EVM).
    pub topics: Vec<B256>,
    /// Unindexed log data.
    pub data: Bytes,
}

impl bincode::Encode for JournalLog {
    fn encode<E: Encoder>(&self, e: &mut E) -> Result<(), EncodeError> {
        bincode::Encode::encode(&(self.topics.len() as u32), e)?;
        for topic in self.topics.iter() {
            bincode::Encode::encode(&topic.0, e)?;
        }
        bincode::Encode::encode(self.data.as_ref(), e)?;
        Ok(())
    }
}

impl<C> bincode::Decode<C> for JournalLog {
    fn decode<D: Decoder<Context = C>>(d: &mut D) -> Result<Self, DecodeError> {
        let topics_len: u32 = bincode::Decode::decode(d)?;
        let mut topics = Vec::with_capacity(topics_len as usize);
        for _ in 0..topics_len {
            let topic: [u8; 32] = bincode::Decode::decode(d)?;
            topics.push(B256::new(topic));
        }
        let data: Vec<u8> = bincode::Decode::decode(d)?;
        Ok(JournalLog {
            topics,
            data: data.into(),
        })
    }
}

/// Storage journal with overlay semantics.
///
/// This structure represents *one execution context* (e.g. a call frame):
/// - `state` is the immutable base storage snapshot
/// - `dirty_values` is the write overlay (changes only)
/// - `events` are logs emitted during execution
///
/// No writes are applied directly to `state`.
/// Instead, diffs are accumulated and later committed or discarded.
#[derive(Default)]
pub struct JournalStorage {
    /// Base storage state (read-only snapshot).
    state: HashMap<U256, U256>,

    /// Storage writes performed during execution.
    ///
    /// Contains only keys whose values differ from `state`.
    dirty_values: HashMap<U256, U256>,

    /// Accumulated event logs (LOG opcodes).
    events: Vec<JournalLog>,
}

impl JournalStorage {
    /// Create a new storage journal from a base state snapshot.
    ///
    /// The provided `state` is treated as immutable for the lifetime
    /// of this journal instance.
    pub fn new(state: HashMap<U256, U256>) -> Self {
        Self {
            state,
            dirty_values: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Read a storage slot using overlay semantics.
    ///
    /// Resolution order:
    /// 1. If the slot was written during execution, return the dirty value.
    /// 2. Otherwise, return the value from the base state.
    /// 3. If neither exists, return `None`.
    ///
    /// This matches EVM `SLOAD` behavior within a transaction.
    pub fn storage(&self, slot: &U256) -> Option<&U256> {
        self.dirty_values.get(slot).or_else(|| self.state.get(slot))
    }

    /// Write a storage slot with journaling semantics.
    ///
    /// Rules:
    /// - If `value` differs from the base state, record it in `dirty_values`
    /// - If `value` equals the base state, remove any existing dirty entry
    ///
    /// This ensures the diff contains *only real changes*.
    pub fn write_storage(&mut self, key: U256, value: U256) {
        let equals_base = self.state.get(&key).filter(|v| *v == &value).is_some();
        if !equals_base {
            self.dirty_values.insert(key, value);
        } else {
            self.dirty_values.remove(&key);
        }
    }

    /// Emit an EVM-style log entry.
    ///
    /// This records a LOG opcode effect during execution.
    /// Logs are accumulated in-order and returned on commit via `into_diff`.
    ///
    /// No validation is performed here:
    /// - topic count limits
    /// - gas accounting
    /// - memory bounds
    /// are expected to be handled by the caller.
    pub fn emit_log(&mut self, topics: Vec<B256>, data: Bytes) {
        self.events.push(JournalLog { topics, data });
    }

    /// Consume the journal and return the execution diff.
    ///
    /// Returns:
    /// - `HashMap<U256, U256>`: storage changes only (no unchanged slots)
    /// - `Vec<JournalLog>`: accumulated event logs
    ///
    /// Intended to be used at commit time (e.g. end of transaction or call).
    pub fn into_diff(self) -> (HashMap<U256, U256>, Vec<JournalLog>) {
        (self.dirty_values, self.events)
    }

    /// Clear all stored state.
    ///
    /// This removes:
    /// - base state snapshot
    /// - dirty overlay
    /// - accumulated logs
    ///
    /// Primarily useful for reuse in tests or pooled execution contexts.
    pub fn clear(&mut self) {
        self.state.clear();
        self.dirty_values.clear();
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashbrown::HashMap;

    #[test]
    fn storage_reads_base_and_dirty() {
        let mut base = HashMap::new();
        base.insert(U256::from(1u64), U256::from(10u64));

        let mut js = JournalStorage::new(base);

        // Reads from base state.
        assert_eq!(js.storage(&U256::from(1u64)), Some(&U256::from(10u64)));
        assert_eq!(js.storage(&U256::from(2u64)), None);

        // Reads from dirty overlay after write.
        js.write_storage(U256::from(1u64), U256::from(11u64));
        assert_eq!(js.storage(&U256::from(1u64)), Some(&U256::from(11u64)));
    }

    #[test]
    fn write_storage_tracks_only_changes() {
        let mut base = HashMap::new();
        base.insert(U256::from(1u64), U256::from(10u64));

        let mut js = JournalStorage::new(base);

        // Writing the same value as base should not create a dirty entry.
        js.write_storage(U256::from(1u64), U256::from(10u64));
        assert!(js.dirty_values.is_empty());

        // Writing a different value creates a dirty entry.
        js.write_storage(U256::from(1u64), U256::from(99u64));
        assert_eq!(
            js.dirty_values.get(&U256::from(1u64)),
            Some(&U256::from(99u64))
        );

        // Writing back to base clears the dirty entry.
        js.write_storage(U256::from(1u64), U256::from(10u64));
        assert!(!js.dirty_values.contains_key(&U256::from(1u64)));
    }

    #[test]
    fn into_diff_returns_dirty_and_logs() {
        let mut base = HashMap::new();
        base.insert(U256::from(1u64), U256::from(10u64));

        let mut js = JournalStorage::new(base);
        js.write_storage(U256::from(1u64), U256::from(11u64));

        let (diff, logs) = js.into_diff();
        assert_eq!(diff.get(&U256::from(1u64)), Some(&U256::from(11u64)));
        assert!(logs.is_empty());
    }

    #[test]
    fn clear_resets_everything() {
        let mut base = HashMap::new();
        base.insert(U256::from(1u64), U256::from(10u64));

        let mut js = JournalStorage::new(base);
        js.write_storage(U256::from(1u64), U256::from(11u64));
        js.events.push(JournalLog::default());

        js.clear();

        assert!(js.state.is_empty());
        assert!(js.dirty_values.is_empty());
        assert!(js.events.is_empty());
    }

    #[test]
    fn emit_log_appends_log() {
        let js = &mut JournalStorage::default();

        let topics = vec![B256::from([1u8; 32]), B256::from([2u8; 32])];
        let data = Bytes::from_static(b"hello");

        js.emit_log(topics.clone(), data.clone());

        assert_eq!(js.events.len(), 1);
        let log = &js.events[0];
        assert_eq!(log.topics, topics);
        assert_eq!(log.data, data);
    }
}
