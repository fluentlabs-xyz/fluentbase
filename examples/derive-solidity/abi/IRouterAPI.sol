// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IRouterApi {

    function greeting(string calldata message) external view returns (string calldata return_0);
    function customGreeting(string calldata message) external view returns (string calldata return_0);
}
