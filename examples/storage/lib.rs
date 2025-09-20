// #![cfg_attr(not(feature = "std"), no_std, no_main)]
// #![allow(dead_code)]
// extern crate alloc;
// extern crate fluentbase_sdk;

// use alloc::{string::String, vec::Vec};
// use alloy_sol_types::{sol, SolEvent};
// use fluentbase_sdk::{
//     basic_entrypoint,
//     codec::Codec,
//     derive::{router, solidity_storage, Contract},
//     Address, ContextReader, SharedAPI, B256, U256,
// };

// pub trait ERC20API {
//     fn constructor(&mut self, name: String, symbol: String, total_supply: U256);
//     fn symbol(&self) -> String;
//     fn name(&self) -> String;
//     fn decimals(&self) -> U256;
//     fn total_supply(&self) -> U256;
//     fn balance_of(&self, account: Address) -> U256;
//     fn transfer(&mut self, to: Address, value: U256) -> U256;
//     fn allowance(&self, owner: Address, spender: Address) -> U256;
//     fn approve(&mut self, spender: Address, value: U256) -> U256;
//     fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256;
// }

// #[derive(Codec, Debug, Clone)]
// struct ERC20ConstructorArgs {
//     name: String,
//     symbol: String,
//     initial_supply: U256,
// }

// // Define the Transfer and Approval events
// sol! {
//     event Transfer(address indexed from, address indexed to, uint256 value);
//     event Approval(address indexed owner, address indexed spender, uint256 value);
// }

// fn emit_event<SDK: SharedAPI, T: SolEvent>(sdk: &mut SDK, event: T) {
//     let data = event.encode_data();
//     let topics: Vec<B256> = event
//         .encode_topics()
//         .iter()
//         .map(|v| B256::from(v.0))
//         .collect();
//     sdk.emit_log(&topics, &data);
// }

// solidity_storage! {
//     U256 InitialSupply;
//     String TokenName;
//     String TokenSymbol;
//     mapping(Address => U256) Balance;
//     mapping(Address => mapping(Address => U256)) Allowance;
// }

// impl Balance {
//     fn add<SDK: SharedAPI>(
//         sdk: &mut SDK,
//         address: Address,
//         amount: U256,
//     ) -> Result<(), &'static str> {
//         let current_balance = Self::get(sdk, address);
//         let new_balance = current_balance + amount;
//         Self::set(sdk, address, new_balance);
//         Ok(())
//     }
//     fn subtract<SDK: SharedAPI>(
//         sdk: &mut SDK,
//         address: Address,
//         amount: U256,
//     ) -> Result<(), &'static str> {
//         let current_balance = Self::get(sdk, address);
//         if current_balance < amount {
//             return Err("insufficient balance");
//         }
//         let new_balance = current_balance - amount;
//         Self::set(sdk, address, new_balance);
//         Ok(())
//     }
// }

// impl Allowance {
//     fn add<SDK: SharedAPI>(
//         sdk: &mut SDK,
//         owner: Address,
//         spender: Address,
//         amount: U256,
//     ) -> Result<(), &'static str> {
//         let current_allowance = Self::get(sdk, owner, spender);
//         let new_allowance = current_allowance + amount;
//         Self::set(sdk, owner, spender, new_allowance);
//         Ok(())
//     }
//     fn subtract<SDK: SharedAPI>(
//         sdk: &mut SDK,
//         owner: Address,
//         spender: Address,
//         amount: U256,
//     ) -> Result<(), &'static str> {
//         let current_allowance = Self::get(sdk, owner, spender);
//         if current_allowance < amount {
//             return Err("insufficient allowance");
//         }
//         let new_allowance = current_allowance - amount;
//         Self::set(sdk, owner, spender, new_allowance);
//         Ok(())
//     }
// }

// #[derive(Contract, Default)]
// struct ERC20<SDK> {
//     sdk: SDK,
// }

// #[router(mode = "solidity")]
// impl<SDK: SharedAPI> ERC20API for ERC20<SDK> {
//     fn constructor(&mut self, name: String, symbol: String, supply: U256) {
//         TokenSymbol::set(&mut self.sdk, symbol);
//         TokenName::set(&mut self.sdk, name);
//         InitialSupply::set(&mut self.sdk, supply);
//         let owner_address = self.sdk.context().contract_caller();
//         let _ = Balance::add(&mut self.sdk, owner_address, supply);
//     }

//     fn symbol(&self) -> String {
//         TokenSymbol::get(&self.sdk)
//     }

//     fn name(&self) -> String {
//         TokenName::get(&self.sdk)
//     }

//     fn decimals(&self) -> U256 {
//         U256::from(18)
//     }

//     fn total_supply(&self) -> U256 {
//         InitialSupply::get(&self.sdk)
//     }

//     fn balance_of(&self, account: Address) -> U256 {
//         Balance::get(&self.sdk, account)
//     }

//     fn transfer(&mut self, to: Address, value: U256) -> U256 {
//         let from = self.sdk.context().contract_caller();

//         Balance::subtract(&mut self.sdk, from, value).unwrap_or_else(|err| panic!("{}", err));
//         Balance::add(&mut self.sdk, to, value).unwrap_or_else(|err| panic!("{}", err));

//         emit_event(&mut self.sdk, Transfer { from, to, value });
//         U256::from(1)
//     }

//     fn allowance(&self, owner: Address, spender: Address) -> U256 {
//         Allowance::get(&self.sdk, owner, spender)
//     }

//     fn approve(&mut self, spender: Address, value: U256) -> U256 {
//         let owner = self.sdk.context().contract_caller();
//         Allowance::set(&mut self.sdk, owner, spender, value);
//         emit_event(
//             &mut self.sdk,
//             Approval {
//                 owner,
//                 spender,
//                 value,
//             },
//         );
//         U256::from(1)
//     }

//     fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> U256 {
//         let spender = self.sdk.context().contract_caller();

//         let current_allowance = Allowance::get(&self.sdk, from, spender);
//         if current_allowance < value {
//             panic!("insufficient allowance");
//         }

//         Allowance::subtract(&mut self.sdk, from, spender, value)
//             .unwrap_or_else(|err| panic!("{}", err));
//         Balance::subtract(&mut self.sdk, from, value).unwrap_or_else(|err| panic!("{}", err));
//         Balance::add(&mut self.sdk, to, value).unwrap_or_else(|err| panic!("{}", err));

//         emit_event(&mut self.sdk, Transfer { from, to, value });
//         U256::from(1)
//     }
// }

// basic_entrypoint!(ERC20);

// #[cfg(test)]
// mod tests {
//     use std::io::Read;

//     use super::*;
//     use fluentbase_sdk::{address, bytes::BytesMut, hex, ContractContextV1, U256};
//     use fluentbase_sdk::codec::SolidityABI;
//     use fluentbase_testing::HostTestingContext;

//     #[test]
//     fn test_erc20_deploy_constructor_args() {
//         let mut buf = BytesMut::new();
//         let args: (String, String, U256) =
//             ("MyToken".to_string(), "MTK".to_string(), U256::from(1_000_000u64));

//         SolidityABI::encode(&args, &mut buf, 0).unwrap();

//         let input = buf.freeze();
//         println!("input {:?}", hex::encode(&input));

//         let context = ContractContextV1 {
//             address: address!("1111111111111111111111111111111111111111"),
//             bytecode_address: address!("2222222222222222222222222222222222222222"),
//             caller: address!("3333333333333333333333333333333333333333"),
//             is_static: false,
//             value: U256::ZERO,
//             gas_limit: 0,
//         };

//         let sdk =
//             HostTestingContext::default().with_input(input).with_contract_context(context.clone());

//         let mut contract = ERC20 { sdk: sdk.clone() };
//         contract.deploy();
//         let storage_symbol = TokenSymbol::get(&mut contract.sdk);
//         println!("storage symbol {:?}", storage_symbol);
//     }
// }
// //
// // #[cfg(test)]
// // mod tests {
// //     use super::*;
// //     use crate::assert_storage_layout;
// //     use fluentbase_sdk::address;
// //     use fluentbase_testing::HostTestingContext;
// //
// //     #[test]
// //     fn test_layout_calculations() {
// //         assert_storage_layout! {
// //             TokenInfo => {
// //                 name: 0, 0,
// //                 symbol: 1, 0,
// //                 decimals: 2, 31,
// //                 total_supply: 3, 0,
// //             },
// //             total_slots: 4
// //         }
// //
// //         assert_storage_layout! {
// //             ERC20State => {
// //                 token_info: 0, 0,
// //                 balances: 4, 0,
// //                 allowances: 5, 0,
// //             },
// //             total_slots: 6
// //         }
// //     }
// //
// //     #[test]
// //     fn test_erc20_deployment() {
// //         let context = ContractContextV1 {
// //             address: address!("1111111111111111111111111111111111111111"),
// //             caller: address!("3333333333333333333333333333333333333333"),
// //             // ... other fields
// //         };
// //
// //         let sdk = HostTestingContext::default().with_contract_context(context);
// //
// //         let mut contract = ERC20::new(sdk, U256::from(0), 0);
// //
// //         // Deploy with constructor
// //         contract.constructor(
// //             "MyToken".to_string(),
// //             "MTK".to_string(),
// //             U256::from(1_000_000),
// //         );
// //
// //         // Verify token info
// //         assert_eq!(contract.name(), "MyToken");
// //         assert_eq!(contract.symbol(), "MTK");
// //         assert_eq!(contract.decimals(), 18);
// //         assert_eq!(contract.total_supply(), U256::from(1_000_000));
// //
// //         // Verify deployer balance
// //         let deployer = address!("3333333333333333333333333333333333333333");
// //         assert_eq!(contract.balance_of(deployer), U256::from(1_000_000));
// //     }
// //
// //     #[test]
// //     fn test_erc20_transfer() {
// //         let sdk = HostTestingContext::default();
// //         let mut contract = ERC20::new(sdk, U256::from(0), 0);
// //
// //         contract.constructor("Test".to_string(), "TST".to_string(), U256::from(1000));
// //
// //         let alice = address!("1111111111111111111111111111111111111111");
// //         let bob = address!("2222222222222222222222222222222222222222");
// //
// //         // Setup: give Alice some tokens
// //         contract
// //             .state()
// //             .balances
// //             .entry(alice)
// //             .set(&mut contract.sdk, U256::from(100));
// //
// //         // Transfer from Alice to Bob
// //         contract.sdk = contract.sdk.with_caller(alice);
// //         assert!(contract.transfer(bob, U256::from(30)));
// //
// //         // Check balances
// //         assert_eq!(contract.balance_of(alice), U256::from(70));
// //         assert_eq!(contract.balance_of(bob), U256::from(30));
// //     }
// // }
