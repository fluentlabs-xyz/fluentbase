#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use fluentbase_sdk::derive::{client, derive_solidity_client, derive_solidity_trait};

derive_solidity_trait!("abi/IRouterAPI.sol");

derive_solidity_trait!(
    interface IRouterApi2 {
        function greeting(string calldata message) external view returns (string calldata return_0);
        function customGreeting(string calldata message) external view returns (string calldata return_0);
    }
);

derive_solidity_client!("abi/IMyProgramWithStruct.sol");
