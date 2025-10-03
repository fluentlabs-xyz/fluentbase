// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Fibonacci {
    function fib(int32 n) public pure returns (int32) {
        if (n <= 0) return 0;
        if (n == 1) return 1;

        int32 a = 0;
        int32 b = 1;

        for (int32 i = 2; i <= n; i++) {
            int32 t = a + b;
            a = b;
            b = t;
        }

        return b;
    }
}
