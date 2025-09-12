#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::{string::String, vec::Vec};
use alloy_sol_types::{sol, SolEvent};
use fluentbase_sdk::{
    derive::{router, Storage},
    entrypoint_with_storage,
    storage::{
        bytes::StorageString,
        composite::Composite,
        map::StorageMap,
        primitive::StoragePrimitive,
        BytesAccess,
        MapAccess,
        PrimitiveAccess,
    },
    Address,
    ContextReader,
    SharedAPI,
    B256,
    U256,
};

// Define the Transfer and Approval events
sol! {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}

fn emit_event<SDK: SharedAPI, T: SolEvent>(sdk: &mut SDK, event: T) {
    let data = event.encode_data();
    let topics: Vec<B256> = event
        .encode_topics()
        .iter()
        .map(|v| B256::from(v.0))
        .collect();
    sdk.emit_log(&topics, &data);
}

// Storage structures
#[derive(Storage)]
pub struct ERC20Storage {
    pub token_name: StorageString,
    pub token_symbol: StorageString,
    pub total_supply: StoragePrimitive<U256>,
    pub balances: StorageMap<Address, StoragePrimitive<U256>>,
    pub allowances: StorageMap<Address, StorageMap<Address, StoragePrimitive<U256>>>,
}

#[derive(Storage)]
pub struct ERC20<SDK> {
    sdk: SDK,
    storage: Composite<ERC20Storage>,
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn constructor(&mut self, name: String, symbol: String, initial_supply: U256) {
        // Set token metadata
        self.storage().token_name().set_string(&mut self.sdk, &name);
        self.storage()
            .token_symbol()
            .set_string(&mut self.sdk, &symbol);
        self.storage()
            .total_supply()
            .set(&mut self.sdk, initial_supply);

        // Assign initial supply to deployer
        let deployer = self.sdk.context().contract_caller();
        self.storage()
            .balances()
            .entry(deployer)
            .set(&mut self.sdk, initial_supply);

        // Emit initial transfer event from zero address
        emit_event(
            &mut self.sdk,
            Transfer {
                from: Address::ZERO,
                to: deployer,
                value: initial_supply,
            },
        );
    }

    pub fn name(&self) -> String {
        self.storage().token_name().get_string(&self.sdk)
    }

    pub fn symbol(&self) -> String {
        self.storage().token_symbol().get_string(&self.sdk)
    }

    pub fn decimals(&self) -> U256 {
        U256::from(18)
    }

    pub fn total_supply(&self) -> U256 {
        self.storage().total_supply().get(&self.sdk)
    }

    pub fn balance_of(&self, account: Address) -> U256 {
        self.storage().balances().entry(account).get(&self.sdk)
    }

    pub fn transfer(&mut self, to: Address, value: U256) -> U256 {
        let from = self.sdk.context().contract_caller();

        // Check sufficient balance
        let from_balance = self.storage().balances().entry(from).get(&self.sdk);
        if from_balance < value {
            panic!("insufficient balance");
        }

        // Update balances
        self.storage()
            .balances()
            .entry(from)
            .set(&mut self.sdk, from_balance - value);

        let to_balance = self.storage().balances().entry(to).get(&self.sdk);
        self.storage()
            .balances()
            .entry(to)
            .set(&mut self.sdk, to_balance + value);

        emit_event(&mut self.sdk, Transfer { from, to, value });
        U256::from(1)
    }

    pub fn allowance(&self, owner: Address, spender: Address) -> U256 {
        self.storage()
            .allowances()
            .entry(owner)
            .entry(spender)
            .get(&self.sdk)
    }

    pub fn approve(&mut self, spender: Address, value: U256) -> U256 {
        let owner = self.sdk.context().contract_caller();

        self.storage()
            .allowances()
            .entry(owner)
            .entry(spender)
            .set(&mut self.sdk, value);

        emit_event(
            &mut self.sdk,
            Approval {
                owner,
                spender,
                value,
            },
        );
        U256::from(1)
    }

    pub fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256 {
        let spender = self.sdk.context().contract_caller();

        // Check allowance
        let current_allowance = self
            .storage()
            .allowances()
            .entry(from)
            .entry(spender)
            .get(&self.sdk);

        if current_allowance < value {
            panic!("insufficient allowance");
        }

        // Check balance
        let from_balance = self.storage().balances().entry(from).get(&self.sdk);
        if from_balance < value {
            panic!("insufficient balance");
        }

        // Update allowance
        self.storage()
            .allowances()
            .entry(from)
            .entry(spender)
            .set(&mut self.sdk, current_allowance - value);

        // Update balances
        self.storage()
            .balances()
            .entry(from)
            .set(&mut self.sdk, from_balance - value);

        let to_balance = self.storage().balances().entry(to).get(&self.sdk);
        self.storage()
            .balances()
            .entry(to)
            .set(&mut self.sdk, to_balance + value);

        emit_event(&mut self.sdk, Transfer { from, to, value });
        U256::from(1)
    }
}

entrypoint_with_storage!(ERC20);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{
        address,
        byteorder,
        bytes::Buf,
        codec::Encoder,
        storage::{BytesAccess, MapAccess, PrimitiveAccess},
        Address,
        ContractContextV1,
        U256,
    };
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_constructor_initializes_correctly() {
        // Arrange
        let deployer = address!("1111111111111111111111111111111111111111");
        let token_name = "MyToken".to_string();
        let token_symbol = "MTK".to_string();
        let initial_supply = U256::from(1_000_000);

        let constructor_call =
            ConstructorCall::new((token_name.clone(), token_symbol.clone(), initial_supply));

        let sdk = HostTestingContext::default()
            .with_input(constructor_call.encode())
            .with_contract_context(ContractContextV1 {
                address: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                caller: deployer,
                ..Default::default()
            });

        // Act
        let mut contract = ERC20::new(sdk.clone(), U256::from(0), 0);
        contract.deploy();

        // Assert - verify storage was initialized correctly
        assert_eq!(
            contract.storage().token_name().get_string(&sdk),
            token_name,
            "Token name not set correctly"
        );

        assert_eq!(
            contract.storage().token_symbol().get_string(&sdk),
            token_symbol,
            "Token symbol not set correctly"
        );

        assert_eq!(
            contract.storage().total_supply().get(&sdk),
            initial_supply,
            "Total supply not set correctly"
        );

        // Verify deployer received initial supply
        assert_eq!(
            contract.storage().balances().entry(deployer).get(&sdk),
            initial_supply,
            "Deployer did not receive initial supply"
        );

        // Verify other addresses have zero balance
        let other_address = address!("2222222222222222222222222222222222222222");
        assert_eq!(
            contract.storage().balances().entry(other_address).get(&sdk),
            U256::ZERO,
            "Non-deployer address should have zero balance"
        );
    }

    #[test]
    fn test_basic_query_functions() {
        // Arrange
        let deployer = address!("1111111111111111111111111111111111111111");
        let token_name = "TestToken".to_string();
        let token_symbol = "TST".to_string();
        let initial_supply = U256::from(10_000_000);

        // Initialize contract with constructor
        let constructor_call =
            ConstructorCall::new((token_name.clone(), token_symbol.clone(), initial_supply));

        let sdk = HostTestingContext::default()
            .with_input(constructor_call.encode())
            .with_contract_context(ContractContextV1 {
                address: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                caller: deployer,
                ..Default::default()
            });

        let mut contract = ERC20::new(sdk.clone(), U256::from(0), 0);
        contract.deploy();

        // Act & Assert - Test name() getter
        contract.sdk = contract.sdk.with_input(NameCall::new(()).encode());
        contract.main();
        let name_result = NameReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            name_result.0 .0, token_name,
            "name() returned incorrect value"
        );

        // Test symbol() getter
        contract.sdk = contract.sdk.with_input(SymbolCall::new(()).encode());
        contract.main();
        let symbol_result = SymbolReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            symbol_result.0 .0, token_symbol,
            "symbol() returned incorrect value"
        );

        // Test decimals() getter
        contract.sdk = contract.sdk.with_input(DecimalsCall::new(()).encode());
        contract.main();
        let decimals_result = DecimalsReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            decimals_result.0 .0,
            U256::from(18),
            "decimals() should return 18"
        );

        // Test total_supply() getter
        contract.sdk = contract.sdk.with_input(TotalSupplyCall::new(()).encode());
        contract.main();
        let total_supply_result =
            TotalSupplyReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            total_supply_result.0 .0, initial_supply,
            "total_supply() returned incorrect value"
        );

        //  ----> current
        // Test balance_of() for deployer
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((deployer,)).encode());
        contract.main();
        let balance_result = BalanceOfReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            balance_result.0 .0, initial_supply,
            "balance_of(deployer) should equal initial supply"
        );

        // Test balance_of() for zero address
        let zero_address = Address::ZERO;
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((zero_address,)).encode());
        contract.main();
        let zero_balance_result =
            BalanceOfReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            zero_balance_result.0 .0,
            U256::ZERO,
            "balance_of(0x0) should return 0"
        );

        // Test balance_of() for random address
        let random_address = address!("9999999999999999999999999999999999999999");
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((random_address,)).encode());
        contract.main();
        let random_balance_result =
            BalanceOfReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            random_balance_result.0 .0,
            U256::ZERO,
            "balance_of(random_address) should return 0"
        );
    }

    #[test]
    fn test_balance_of_bug() {
        // Arrange
        let deployer = address!("1111111111111111111111111111111111111111");
        let token_name = "TestToken".to_string();
        let token_symbol = "TST".to_string();
        let initial_supply = U256::from(10_000_000);

        // Initialize contract with constructor
        let constructor_call =
            ConstructorCall::new((token_name.clone(), token_symbol.clone(), initial_supply));

        let sdk = HostTestingContext::default()
            .with_input(constructor_call.encode())
            .with_contract_context(ContractContextV1 {
                address: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                caller: deployer,
                ..Default::default()
            });

        let mut contract = ERC20::new(sdk.clone(), U256::from(0), 0);
        contract.deploy();

        let balance_of_input = BalanceOfCall::new((deployer,)).encode();
        println!("{:x}", &balance_of_input);

        println!(
            "Address IS_DYNAMIC: {}",
            <Address as Encoder<byteorder::BE, 32, true, true>>::IS_DYNAMIC
        );

        // Check if the tuple (Address,) is dynamic
        println!(
            "(Address,) IS_DYNAMIC: {}",
            <(Address,) as Encoder<byteorder::BE, 32, true, true>>::IS_DYNAMIC
        );

        let decoded = BalanceOfCall::decode(&&balance_of_input.chunk()[4..]).unwrap();

        println!("{:?}", decoded);
    }

    #[test]
    fn test_transfer_functionality() {
        // Arrange
        let sender = address!("1111111111111111111111111111111111111111");
        let recipient = address!("2222222222222222222222222222222222222222");
        let token_address = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let initial_supply = U256::from(1_000_000);
        let transfer_amount = U256::from(100_000);

        let constructor_call =
            ConstructorCall::new(("TestToken".to_string(), "TST".to_string(), initial_supply));

        let sdk = HostTestingContext::default()
            .with_input(constructor_call.encode())
            .with_contract_context(ContractContextV1 {
                address: token_address,
                caller: sender,
                ..Default::default()
            });

        let mut contract = ERC20::new(sdk.clone(), U256::from(0), 0);
        contract.deploy();

        // Act - Transfer tokens from sender to recipient
        contract.sdk = contract
            .sdk
            .with_input(TransferCall::new((recipient, transfer_amount)).encode())
            .with_contract_context(ContractContextV1 {
                address: token_address,
                caller: sender,
                ..Default::default()
            });

        contract.main();
        let transfer_result = TransferReturn::decode(&&contract.sdk.take_output()[..]).unwrap();

        // Assert - Check return value
        assert_eq!(
            transfer_result.0 .0,
            U256::from(1),
            "transfer should return 1 on success"
        );

        // Verify sender balance decreased
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((sender,)).encode());
        contract.main();
        let sender_balance = BalanceOfReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            sender_balance.0 .0,
            initial_supply - transfer_amount,
            "sender balance should decrease by transfer amount"
        );

        // Verify recipient balance increased
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((recipient,)).encode());
        contract.main();
        let recipient_balance = BalanceOfReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            recipient_balance.0 .0, transfer_amount,
            "recipient balance should equal transfer amount"
        );

        // Test edge case: transfer zero amount
        contract.sdk = contract
            .sdk
            .with_input(TransferCall::new((recipient, U256::ZERO)).encode())
            .with_contract_context(ContractContextV1 {
                address: token_address,
                caller: sender,
                ..Default::default()
            });

        contract.main();
        let zero_transfer_result =
            TransferReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            zero_transfer_result.0 .0,
            U256::from(1),
            "transfer of 0 should succeed"
        );

        // Get actual current balance before transferring all
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((sender,)).encode());
        contract.main();
        let current_sender_balance = BalanceOfReturn::decode(&&contract.sdk.take_output()[..])
            .unwrap()
            .0
             .0;

        // Test transfer entire balance using the actual current balance
        contract.sdk = contract
            .sdk
            .with_input(TransferCall::new((recipient, current_sender_balance)).encode())
            .with_contract_context(ContractContextV1 {
                address: token_address,
                caller: sender,
                ..Default::default()
            });

        contract.main();
        let entire_balance_transfer_result =
            TransferReturn::decode(&&contract.sdk.take_output()[..]).unwrap();
        assert_eq!(
            entire_balance_transfer_result.0 .0,
            U256::from(1),
            "transfer of 0 should succeed"
        );

        // Verify sender now has 0
        contract.sdk = contract
            .sdk
            .with_input(BalanceOfCall::new((sender,)).encode());
        contract.main();
        let output = contract.sdk.take_output();
        let final_sender_balance = BalanceOfReturn::decode(&&output[..]).unwrap();

        assert_eq!(
            final_sender_balance.0 .0,
            U256::ZERO,
            "sender should have zero balance after transferring all"
        );
    }

    #[test]
    fn print_constructor_params_hex() {
        let constructor_params = ConstructorCall::new((
            "TestToken".to_string(),
            "TST".to_string(),
            U256::from(1_000_000),
        ))
        .encode();
        println!("Constructor input: {:x}", &constructor_params);
    }
}
