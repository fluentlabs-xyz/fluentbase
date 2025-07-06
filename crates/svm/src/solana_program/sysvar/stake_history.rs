pub use crate::solana_program::stake_history::StakeHistory;
use crate::solana_program::{
    stake_history::{StakeHistoryEntry, StakeHistoryGetEntry, MAX_ENTRIES},
    sysvar::{get_sysvar, Sysvar, SysvarId},
};
use solana_bincode::deserialize;
use solana_clock::Epoch;
use solana_sysvar_id::declare_sysvar_id;

declare_sysvar_id!("SysvarStakeHistory1111111111111111111111111", StakeHistory);

impl Sysvar for StakeHistory {
    // override
    fn size_of() -> usize {
        // hard-coded so that we don't have to construct an empty
        16392 // golden, update if MAX_ENTRIES changes
    }
}

// we do not provide Default because this requires the real current epoch
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct StakeHistorySysvar(pub Epoch);

// precompute so we can statically allocate buffer
const EPOCH_AND_ENTRY_SERIALIZED_SIZE: u64 = 32;

impl StakeHistoryGetEntry for StakeHistorySysvar {
    fn get_entry(&self, target_epoch: Epoch) -> Option<StakeHistoryEntry> {
        let current_epoch = self.0;

        // if current epoch is zero this returns None because there is no history yet
        let newest_historical_epoch = current_epoch.checked_sub(1)?;
        let oldest_historical_epoch = current_epoch.saturating_sub(MAX_ENTRIES as u64);

        // target epoch is old enough to have fallen off history; presume fully active/deactive
        if target_epoch < oldest_historical_epoch {
            return None;
        }

        // epoch delta is how many epoch-entries we offset in the stake history vector, which may be zero
        // None means target epoch is current or in the future; this is a user error
        let epoch_delta = newest_historical_epoch.checked_sub(target_epoch)?;

        // offset is the number of bytes to our desired entry, including eight for vector length
        let offset = epoch_delta
            .checked_mul(EPOCH_AND_ENTRY_SERIALIZED_SIZE)?
            .checked_add(core::mem::size_of::<u64>() as u64)?;

        let mut entry_buf = [0; EPOCH_AND_ENTRY_SERIALIZED_SIZE as usize];
        let result = get_sysvar(
            &mut entry_buf,
            &StakeHistory::id(),
            offset,
            EPOCH_AND_ENTRY_SERIALIZED_SIZE,
        );

        match result {
            Ok(()) => {
                let (entry_epoch, entry) =
                    deserialize::<(Epoch, StakeHistoryEntry)>(&entry_buf).ok()?;

                // this would only fail if stake history skipped an epoch or the binary format of the sysvar changed
                assert_eq!(entry_epoch, target_epoch);

                Some(entry)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana_program::{stake_history::*, sysvar::tests::mock_get_sysvar_syscall};
    use serial_test::serial;
    use solana_bincode::{serialize, serialized_size};

    #[test]
    fn test_size_of() {
        let mut stake_history = StakeHistory::default();
        for i in 0..MAX_ENTRIES as u64 {
            stake_history.add(
                i,
                StakeHistoryEntry {
                    activating: i,
                    ..StakeHistoryEntry::default()
                },
            );
        }

        assert_eq!(
            serialized_size(&stake_history).unwrap(),
            StakeHistory::size_of()
        );

        let stake_history_inner: Vec<(Epoch, StakeHistoryEntry)> =
            deserialize(&serialize(&stake_history).unwrap()).unwrap();
        let epoch_entry = stake_history_inner.into_iter().next().unwrap();

        assert_eq!(
            serialized_size(&epoch_entry).unwrap() as u64,
            EPOCH_AND_ENTRY_SERIALIZED_SIZE
        );
    }

    #[serial]
    #[test]
    fn test_stake_history_get_entry() {
        let unique_entry_for_epoch = |epoch: u64| StakeHistoryEntry {
            activating: epoch.saturating_mul(2),
            deactivating: epoch.saturating_mul(3),
            effective: epoch.saturating_mul(5),
        };

        let current_epoch = MAX_ENTRIES.saturating_add(2) as u64;

        // make a stake history object with at least one valid entry that has expired
        let mut stake_history = StakeHistory::default();
        for i in 0..current_epoch {
            stake_history.add(i, unique_entry_for_epoch(i));
        }
        assert_eq!(stake_history.len(), MAX_ENTRIES);
        assert_eq!(stake_history.iter().map(|entry| entry.0).min().unwrap(), 2);

        // set up sol_get_sysvar
        mock_get_sysvar_syscall(&serialize(&stake_history).unwrap());

        // make a syscall interface object
        let stake_history_sysvar = StakeHistorySysvar(current_epoch);

        // now test the stake history interfaces

        assert_eq!(stake_history.get(0), None);
        assert_eq!(stake_history.get(1), None);
        assert_eq!(stake_history.get(current_epoch), None);

        assert_eq!(stake_history.get_entry(0), None);
        assert_eq!(stake_history.get_entry(1), None);
        assert_eq!(stake_history.get_entry(current_epoch), None);

        assert_eq!(stake_history_sysvar.get_entry(0), None);
        assert_eq!(stake_history_sysvar.get_entry(1), None);
        assert_eq!(stake_history_sysvar.get_entry(current_epoch), None);

        for i in 2..current_epoch {
            let entry = Some(unique_entry_for_epoch(i));

            assert_eq!(stake_history.get(i), entry.as_ref(),);

            assert_eq!(stake_history.get_entry(i), entry,);

            assert_eq!(stake_history_sysvar.get_entry(i), entry,);
        }
    }

    #[serial]
    #[test]
    fn test_stake_history_get_entry_zero() {
        let mut current_epoch = 0;

        // first test that an empty history returns None
        let stake_history = StakeHistory::default();
        assert_eq!(stake_history.len(), 0);

        mock_get_sysvar_syscall(&serialize(&stake_history).unwrap());
        let stake_history_sysvar = StakeHistorySysvar(current_epoch);

        assert_eq!(stake_history.get(0), None);
        assert_eq!(stake_history.get_entry(0), None);
        assert_eq!(stake_history_sysvar.get_entry(0), None);

        // next test that we can get a zeroth entry in the first epoch
        let entry_zero = StakeHistoryEntry {
            effective: 100,
            ..StakeHistoryEntry::default()
        };
        let entry = Some(entry_zero.clone());

        let mut stake_history = StakeHistory::default();
        stake_history.add(current_epoch, entry_zero);
        assert_eq!(stake_history.len(), 1);
        current_epoch = current_epoch.saturating_add(1);

        mock_get_sysvar_syscall(&serialize(&stake_history).unwrap());
        let stake_history_sysvar = StakeHistorySysvar(current_epoch);

        assert_eq!(stake_history.get(0), entry.as_ref());
        assert_eq!(stake_history.get_entry(0), entry);
        assert_eq!(stake_history_sysvar.get_entry(0), entry);

        // finally test that we can still get a zeroth entry in later epochs
        stake_history.add(current_epoch, StakeHistoryEntry::default());
        assert_eq!(stake_history.len(), 2);
        current_epoch = current_epoch.saturating_add(1);

        mock_get_sysvar_syscall(&serialize(&stake_history).unwrap());
        let stake_history_sysvar = StakeHistorySysvar(current_epoch);

        assert_eq!(stake_history.get(0), entry.as_ref());
        assert_eq!(stake_history.get_entry(0), entry);
        assert_eq!(stake_history_sysvar.get_entry(0), entry);
    }
}
