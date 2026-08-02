// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// Harness-local load-generator fixture (devnet/local-dpos-smoke/scripts/load-heavy.sh).
/// burn(n) performs n SSTOREs into FRESH mapping slots (cold zero->nonzero writes,
/// ~22k gas each) — SSTORE-heavy, state-growth-realistic execution load. The cursor
/// guarantees every call touches new slots, so per-iteration gas stays constant
/// across the whole run (no warm-slot decay of the calibration).
contract GasBurner {
    mapping(uint256 => uint256) private slots;
    uint256 private cursor;

    function burn(uint256 n) external {
        uint256 c = cursor;
        for (uint256 i = 0; i < n; ++i) {
            slots[c + i] = block.number + i;
        }
        cursor = c + n;
    }
}
