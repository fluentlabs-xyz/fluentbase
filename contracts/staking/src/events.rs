use fluentbase_sdk::{derive::Event, Address};

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
