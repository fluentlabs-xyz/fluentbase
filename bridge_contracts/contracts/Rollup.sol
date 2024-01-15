import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";


contract Rollup is OwnableUpgradeable {

    uint256 public lastProofedIndex;

    mapping(uint256 => bytes32) public withdrawRoots;
    mapping(uint256 => bytes32) public depositRoots;

    function acceptNextProof(
        uint256 _proofIndex,
        bytes32 _withdrawRoot
    ) external payable {
        require(lastProofedIndex + 1 == _proofIndex, "Incorrect proof index");

        withdrawRoots[_proofIndex] = _withdrawRoot;
        lastProofedIndex = _proofIndex;
    }

    function acceptedProofIndex(
        uin256 _proofIndex
    ) external view returns (bool) {
        return _proofIndex <= lastProofedIndex;
    }

    function verifyMerkleProof(
        bytes32 _root,
        bytes32 _hash,
        uint256 _nonce,
        bytes memory _proof
    ) internal pure returns (bool) {
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
