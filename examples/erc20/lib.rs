#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{constructor, router, Contract, Event},
    storage::{StorageMap, StorageString, StorageU256},
    Address, ContextReader, SharedAPI, U256,
};

/// ERC20 Transfer event
#[derive(Event)]
struct Transfer {
    #[indexed]
    from: Address,
    #[indexed]
    to: Address,
    value: U256,
}

/// ERC20 Approval event
#[derive(Event)]
struct Approval {
    #[indexed]
    owner: Address,
    #[indexed]
    spender: Address,
    value: U256,
}

pub trait ERC20Interface {
    fn name(&self) -> String;
    fn symbol(&self) -> String;
    fn decimals(&self) -> U256;
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> U256;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&mut self, spender: Address, value: U256) -> U256;
    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256;
}

#[derive(Contract)]
pub struct ERC20<SDK> {
    sdk: SDK,
    token_name: StorageString,
    token_symbol: StorageString,
    total_supply: StorageU256,
    balances: StorageMap<Address, StorageU256>,
    allowances: StorageMap<Address, StorageMap<Address, StorageU256>>,
}

#[constructor(mode = "solidity")]
impl<SDK: SharedAPI> ERC20<SDK> {
    pub fn constructor(&mut self, name: String, symbol: String, initial_supply: U256) {
        self.token_name_accessor().set(&mut self.sdk, &name);
        self.token_symbol_accessor().set(&mut self.sdk, &symbol);
        self.total_supply_accessor()
            .set(&mut self.sdk, initial_supply);

        let deployer = self.sdk.context().contract_caller();
        self.balances_accessor()
            .entry(deployer)
            .set(&mut self.sdk, initial_supply);

        Transfer {
            from: Address::ZERO,
            to: deployer,
            value: initial_supply,
        }
        .emit(&mut self.sdk);
    }
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ERC20Interface for ERC20<SDK> {
    fn name(&self) -> String {
        self.token_name_accessor().get(&self.sdk)
    }

    fn symbol(&self) -> String {
        self.token_symbol_accessor().get(&self.sdk)
    }

    fn decimals(&self) -> U256 {
        U256::from(18)
    }

    fn total_supply(&self) -> U256 {
        self.total_supply_accessor().get(&self.sdk)
    }

    fn balance_of(&self, account: Address) -> U256 {
        self.balances_accessor().entry(account).get(&self.sdk)
    }

    fn transfer(&mut self, to: Address, value: U256) -> U256 {
        let from = self.sdk.context().contract_caller();

        let from_balance = self.balances_accessor().entry(from).get(&self.sdk);
        if from_balance < value {
            panic!("insufficient balance");
        }

        self.balances_accessor()
            .entry(from)
            .set(&mut self.sdk, from_balance - value);

        let to_balance = self.balances_accessor().entry(to).get(&self.sdk);
        self.balances_accessor()
            .entry(to)
            .set(&mut self.sdk, to_balance + value);

        Transfer { from, to, value }.emit(&mut self.sdk);
        U256::from(1)
    }

    fn allowance(&self, owner: Address, spender: Address) -> U256 {
        self.allowances_accessor()
            .entry(owner)
            .entry(spender)
            .get(&self.sdk)
    }

    fn approve(&mut self, spender: Address, value: U256) -> U256 {
        let owner = self.sdk.context().contract_caller();

        self.allowances_accessor()
            .entry(owner)
            .entry(spender)
            .set(&mut self.sdk, value);

        Approval {
            owner,
            spender,
            value,
        }
        .emit(&mut self.sdk);
        U256::from(1)
    }

    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256 {
        let spender = self.sdk.context().contract_caller();

        let current_allowance = self
            .allowances_accessor()
            .entry(from)
            .entry(spender)
            .get(&self.sdk);

        if current_allowance < value {
            panic!("insufficient allowance");
        }

        let from_balance = self.balances_accessor().entry(from).get(&self.sdk);
        if from_balance < value {
            panic!("insufficient balance");
        }

        self.allowances_accessor()
            .entry(from)
            .entry(spender)
            .set(&mut self.sdk, current_allowance - value);

        self.balances_accessor()
            .entry(from)
            .set(&mut self.sdk, from_balance - value);

        let to_balance = self.balances_accessor().entry(to).get(&self.sdk);
        self.balances_accessor()
            .entry(to)
            .set(&mut self.sdk, to_balance + value);

        Transfer { from, to, value }.emit(&mut self.sdk);
        U256::from(1)
    }
}

basic_entrypoint!(ERC20);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::{address, ContractContextV1, U256};
    use fluentbase_testing::HostTestingContext;

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
        let mut contract = ERC20::new(sdk.clone());
        contract.deploy();

        // Assert - verify storage was initialized correctly
        assert_eq!(
            contract.token_name_accessor().get(&sdk),
            token_name,
            "Token name not set correctly"
        );

        assert_eq!(
            contract.token_symbol_accessor().get(&sdk),
            token_symbol,
            "Token symbol not set correctly"
        );

        assert_eq!(
            contract.total_supply_accessor().get(&sdk),
            initial_supply,
            "Total supply not set correctly"
        );

        // Verify deployer received initial supply
        assert_eq!(
            contract.balances_accessor().entry(deployer).get(&sdk),
            initial_supply,
            "Deployer did not receive initial supply"
        );

        // Verify other addresses have zero balance
        let other_address = address!("2222222222222222222222222222222222222222");
        assert_eq!(
            contract.balances_accessor().entry(other_address).get(&sdk),
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

        let mut contract = ERC20::new(sdk.clone());
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

        let mut contract = ERC20::new(sdk.clone());
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
    }

    mod events {
        use super::*;
        use fluentbase_sdk::address;
        use fluentbase_testing::HostTestingContext;

        /// Known ERC20 Transfer selector: keccak256("Transfer(address,address,uint256)")
        const TRANSFER_SELECTOR: [u8; 32] = [
            0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37,
            0x8d, 0xaa, 0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d,
            0xf5, 0x23, 0xb3, 0xef,
        ];

        /// Known ERC20 Approval selector: keccak256("Approval(address,address,uint256)")
        const APPROVAL_SELECTOR: [u8; 32] = [
            0x8c, 0x5b, 0xe1, 0xe5, 0xeb, 0xec, 0x7d, 0x5b, 0xd1, 0x4f, 0x71, 0x42, 0x7d, 0x1e,
            0x84, 0xf3, 0xdd, 0x03, 0x14, 0xc0, 0xf7, 0xb2, 0x29, 0x1e, 0x5b, 0x20, 0x0a, 0xc8,
            0xc7, 0xc3, 0xb9, 0x25,
        ];

        #[test]
        fn test_event_selectors_match_solidity() {
            // Verify our macro generates correct selectors
            assert_eq!(
                Transfer::SELECTOR,
                TRANSFER_SELECTOR,
                "Transfer selector mismatch"
            );
            assert_eq!(
                Approval::SELECTOR,
                APPROVAL_SELECTOR,
                "Approval selector mismatch"
            );
        }

        #[test]
        fn test_event_signatures() {
            assert_eq!(Transfer::SIGNATURE, "Transfer(address,address,uint256)");
            assert_eq!(Approval::SIGNATURE, "Approval(address,address,uint256)");
        }

        #[test]
        fn test_transfer_event_encoding() {
            let from = address!("1111111111111111111111111111111111111111");
            let to = address!("2222222222222222222222222222222222222222");
            let value = U256::from(1000);

            let mut sdk = HostTestingContext::default();

            Transfer { from, to, value }.emit(&mut sdk);

            let logs = sdk.take_logs();
            assert_eq!(logs.len(), 1);

            let (data, topics) = &logs[0];

            // topics[0] = selector
            assert_eq!(topics[0].0, Transfer::SELECTOR);

            // topics[1] = from (left-padded)
            let mut expected_from = [0u8; 32];
            expected_from[12..32].copy_from_slice(from.as_slice());
            assert_eq!(topics[1].0, expected_from);

            // topics[2] = to (left-padded)
            let mut expected_to = [0u8; 32];
            expected_to[12..32].copy_from_slice(to.as_slice());
            assert_eq!(topics[2].0, expected_to);

            // data = ABI-encoded value
            assert_eq!(data.len(), 32);
            let decoded_value = U256::from_be_slice(data);
            assert_eq!(decoded_value, value);
        }
    }
}
