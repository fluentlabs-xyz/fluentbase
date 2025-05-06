#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::string::String;
use alloy_sol_types::{sol, SolCall};
use core::u64;
use fluentbase_sdk::{
    codec::Codec,
    derive::{client, derive_solidity_client, derive_solidity_trait},
    Address,
    SharedAPI,
    U256,
};

// sol!("abi/IMyProgramWithStruct.sol");

sol!(
    interface IRouter {

        function swap(address tokenIn, address tokenOut, uint amount) external returns (uint);
    }
);

derive_solidity_client!("abi/IRouterAPI.sol");

derive_solidity_trait!(
    interface IRouterApi2 {
        function greeting(string calldata message) external view returns (string calldata return_0);
        function customGreeting(string calldata message) external view returns (string calldata return_0);
    }
);

// derive_solidity_trait!("abi/IMyProgramWithStruct.sol");

#[derive(::fluentbase_sdk::codec::Codec, Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub age: U256,
}
