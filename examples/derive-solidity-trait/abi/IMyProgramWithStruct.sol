// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IMyProgramWithStruct {
    struct Greeting {
        string prefix;
        string name;
    }

    function greeting(Greeting calldata input) external view returns (Greeting calldata return_0);
    function customGreeting(Greeting calldata input) external view returns (Greeting calldata return_0);
}
