import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Bridge} from "./Bridge.sol";

contract Rollup is Ownable {
    address public bridge;

    uint256 public lastProofedIndex;

    mapping(uint256 => bytes32) public withdrawRoots;
    mapping(uint256 => bytes32) public depositsRoots;

    constructor () Ownable(msg.sender) {}

    function setBridge(address _bridge) external payable onlyOwner {
        bridge = _bridge;
    }

    function acceptNextProof(
        uint256 _proofIndex,
        bytes32 _withdrawRoot,
        bytes memory _depositHashes
    ) external payable {
        require(lastProofedIndex + 1 == _proofIndex, "Incorrect proof index");

        if (_depositHashes.length != 0) {
            require(validateDepositsHashes(_depositHashes), "Incorrect deposit hash");
            bytes32 _depositRoot = _calculateMerkleRoot(_depositHashes);
            depositsRoots[_proofIndex] = _depositRoot;
        }

        withdrawRoots[_proofIndex] = _withdrawRoot;
        lastProofedIndex = _proofIndex;
    }

    function acceptedProofIndex(
        uint256 _proofIndex
    ) external view returns (bool) {
        return _proofIndex <= lastProofedIndex;
    }

    function validateDepositsHashes(
        bytes memory _leafs
    ) internal returns (bool) {
        uint256 count = _leafs.length / 32;

        for(uint256 i = 0; i < count; i++) {
            bytes32 messageHash;
            assembly {
                messageHash := mload(add(add(_leafs, 32), mul(i, 32)))
            }
            if (!Bridge(bridge).sentMessage(messageHash)) {
                return false;
            }
        }

        return true;
    }

    function calculateMerkleRoot(
        bytes memory _leafs
    ) external pure returns (bytes32) {
        return _calculateMerkleRoot(_leafs);
    }

    function _calculateMerkleRoot(
        bytes memory _leafs
    ) internal pure returns (bytes32) {
        uint256 count = _leafs.length / 32;

        require(count > 0, "empty leafs");

        while (count > 0) {
            bytes32 hash;
            bytes32 left;
            bytes32 right;
            for(uint256 i = 0; i < count/2; i++) {

                assembly {
                    left := mload(add(add(_leafs, 32), mul(mul(i,2), 32)))
                    right := mload(add(add(_leafs, 32), mul(add(mul(i,2), 1), 32)))
                }
                hash = _efficientHash(left, right);
                assembly {
                    mstore(add(add(_leafs, 32), mul(i, 32)), hash)
                }
            }

            if (count % 2 == 1 && count > 1) {
                assembly {
                    left := mload(add(add(_leafs, 32), mul(sub(count, 1), 32)))
                }
                hash = _efficientHash(left, bytes32(0));

                assembly {
                    mstore(add(add(_leafs, 32), mul(div(sub(count,1),2), 32)), hash)
                }
                count += 1;
            }

            count = count / 2;
        }
        bytes32 root;
        assembly {
          root := mload(add(_leafs, 32))
        }

        return root;
    }

    function verifyMerkleProof(
        bytes32 _root,
        bytes32 _hash,
        uint256 _nonce,
        bytes memory _proof
    ) external pure returns (bool) {
        require(_proof.length % 32 == 0, "Invalid proof");
        uint256 _length = _proof.length / 32;

        for (uint256 i = 0; i < _length; i++) {
            bytes32 item;
            assembly {
                item := mload(add(add(_proof, 32), mul(i, 32)))
            }
            if (_nonce % 2 == 0) {
                _hash = _efficientHash(_hash, item);
            } else {
                _hash = _efficientHash(item, _hash);
            }
            _nonce /= 2;
        }
        return _hash == _root;
    }

    function _efficientHash(bytes32 a, bytes32 b) private pure returns (bytes32 value) {
        assembly {
            mstore(0x00, a)
            mstore(0x20, b)
            value := keccak256(0x00, 0x40)
        }
    }
}
