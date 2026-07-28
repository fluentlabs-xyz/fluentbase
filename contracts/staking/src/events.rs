use fluentbase_sdk::{derive::Event, Address, U256};

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorAdded {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub owner: Address,
    pub status: u8,
    pub commission_rate: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorModified {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub owner: Address,
    pub status: u8,
    pub commission_rate: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorRemoved {
    #[indexed]
    pub validator: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct Delegated {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct Undelegated {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub epoch: u64,
}
