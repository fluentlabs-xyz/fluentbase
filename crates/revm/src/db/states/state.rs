use super::{
    bundle_state::BundleRetention, cache::CacheState, plain_account::PlainStorage, BundleState,
    CacheAccount, StateBuilder, TransitionAccount, TransitionState,
};
use fluentbase_types::{EmptyJournalTrie, ExitCode, IJournaledTrie};
use revm_primitives::{db::Database, AccountInfo, Address, B256};
use std::{boxed::Box, collections::BTreeMap, vec::Vec};

/// Database boxed with a lifetime and Send.
pub type DBBox<'a, E> = Box<dyn Database<Error = E> + Send + 'a>;

/// More constrained version of State that uses Boxed database with a lifetime.
///
/// This is used to make it easier to use State.
pub type StateDBBox<'a, E> = State<DBBox<'a, E>>;

/// State of blockchain.
///
/// State clear flag is set inside CacheState and by default it is enabled.
/// If you want to disable it use `set_state_clear_flag` function.
#[derive(Debug)]
pub struct State<DB> {
    /// Cached state contains both changed from evm execution and cached/loaded account/storages
    /// from database. This allows us to have only one layer of cache where we can fetch data.
    /// Additionally we can introduce some preloading of data from database.
    pub cache: CacheState,
    /// Optional database that we use to fetch data from. If database is not present, we will
    /// return not existing account and storage.
    ///
    /// Note: It is marked as Send so database can be shared between threads.
    pub database: DB,
    /// Block state, it aggregates transactions transitions into one state.
    ///
    /// Build reverts and state that gets applied to the state.
    pub transition_state: Option<TransitionState>,
    /// After block is finishes we merge those changes inside bundle.
    /// Bundle is used to update database and create changesets.
    /// Bundle state can be set on initialization if we want to use preloaded bundle.
    pub bundle_state: BundleState,
    /// Addition layer that is going to be used to fetched values before fetching values
    /// from database.
    ///
    /// Bundle is the main output of the state execution and this allows setting previous bundle
    /// and using its values for execution.
    pub use_preloaded_bundle: bool,
    /// If EVM asks for block hash we will first check if they are found here.
    /// and then ask the database.
    ///
    /// This map can be used to give different values for block hashes if in case
    /// The fork block is different or some blocks are not saved inside database.
    pub block_hashes: BTreeMap<u64, B256>,
}

// Have ability to call State::builder without having to specify the type.
impl State<EmptyJournalTrie> {
    /// Return the builder that build the State.
    pub fn builder() -> StateBuilder<EmptyJournalTrie> {
        StateBuilder::default()
    }
}

impl<DB: Database> State<DB> {
    /// Returns the size hint for the inner bundle state.
    /// See [BundleState::size_hint] for more info.
    pub fn bundle_size_hint(&self) -> usize {
        self.bundle_state.size_hint()
    }

    /// Iterate over received balances and increment all account balances.
    /// If account is not found inside cache state it will be loaded from database.
    ///
    /// Update will create transitions for all accounts that are updated.
    ///
    /// Like [CacheAccount::increment_balance], this assumes that incremented balances are not
    /// zero, and will not overflow once incremented. If using this to implement withdrawals, zero
    /// balances must be filtered out before calling this function.
    pub fn increment_balances(
        &mut self,
        _balances: impl IntoIterator<Item = (Address, u128)>,
    ) -> Result<(), ExitCode> {
        todo!("not implemented")
    }

    /// Drain balances from given account and return those values.
    ///
    /// It is used for DAO hardfork state change to move values from given accounts.
    pub fn drain_balances(
        &mut self,
        _addresses: impl IntoIterator<Item = Address>,
    ) -> Result<Vec<u128>, ExitCode> {
        todo!("not implemented")
    }

    /// State clear EIP-161 is enabled in Spurious Dragon hardfork.
    pub fn set_state_clear_flag(&mut self, has_state_clear: bool) {
        self.cache.set_state_clear_flag(has_state_clear);
    }

    pub fn insert_not_existing(&mut self, address: Address) {
        self.cache.insert_not_existing(address)
    }

    pub fn insert_account(&mut self, address: Address, info: AccountInfo) {
        self.cache.insert_account(address, info)
    }

    pub fn insert_account_with_storage(
        &mut self,
        address: Address,
        info: AccountInfo,
        storage: PlainStorage,
    ) {
        self.cache
            .insert_account_with_storage(address, info, storage)
    }

    /// Apply evm transitions to transition state.
    pub fn apply_transition(&mut self, transitions: Vec<(Address, TransitionAccount)>) {
        // add transition to transition state.
        if let Some(s) = self.transition_state.as_mut() {
            s.add_transitions(transitions)
        }
    }

    /// Take all transitions and merge them inside bundle state.
    /// This action will create final post state and all reverts so that
    /// we at any time revert state of bundle to the state before transition
    /// is applied.
    pub fn merge_transitions(&mut self, retention: BundleRetention) {
        if let Some(transition_state) = self.transition_state.as_mut().map(TransitionState::take) {
            self.bundle_state
                .apply_transitions_and_create_reverts(transition_state, retention);
        }
    }

    // TODO make cache aware of transitions dropping by having global transition counter.
    /// Takes changeset and reverts from state and replaces it with empty one.
    /// This will trop pending Transition and any transitions would be lost.
    ///
    /// NOTE: If either:
    /// * The [State] has not been built with [StateBuilder::with_bundle_update], or
    /// * The [State] has a [TransitionState] set to `None` when
    /// [State::merge_transitions] is called,
    ///
    /// this will panic.
    pub fn take_bundle(&mut self) -> BundleState {
        core::mem::take(&mut self.bundle_state)
    }
}
