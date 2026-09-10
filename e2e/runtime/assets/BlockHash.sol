pragma solidity ^0.8.0;

contract BlockHashTest {
    function getBlockHash(uint256 blockNumber) public view returns (bytes32) {
        return blockhash(blockNumber);
    }

    function getCurrentBlock() public view returns (uint256) {
        return block.number;
    }
}
