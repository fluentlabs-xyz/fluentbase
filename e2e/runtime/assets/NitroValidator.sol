// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15 ^0.8.4;

// src/ICertManager.sol

interface ICertManager {
    struct VerifiedCert {
        bool ca;
        uint64 notAfter;
        int64 maxPathLen;
        bytes32 subjectHash;
        bytes pubKey;
    }

    function verifyCACert(bytes memory cert, bytes32 parentCertHash) external returns (bytes32);

    function verifyClientCert(bytes memory cert, bytes32 parentCertHash) external returns (VerifiedCert memory);
}

// src/LibBytes.sol

library LibBytes {
    function keccak(bytes memory data, uint256 offset, uint256 length) internal pure returns (bytes32 result) {
        require(offset + length <= data.length, "index out of bounds");
        assembly {
            result := keccak256(add(data, add(32, offset)), length)
        }
    }

    function slice(bytes memory b, uint256 offset, uint256 length) internal pure returns (bytes memory result) {
        require(offset + length <= b.length, "index out of bounds");

        // Create a new bytes structure and copy contents
        result = new bytes(length);
        uint256 dest;
        uint256 src;
        assembly {
            dest := add(result, 32)
            src := add(b, add(32, offset))
        }
        memcpy(dest, src, length);
        return result;
    }

    function readUint16(bytes memory b, uint256 index) internal pure returns (uint16) {
        require(b.length >= index + 2, "index out of bounds");
        bytes2 result;
        assembly {
            result := mload(add(b, add(index, 32)))
        }
        return uint16(result);
    }

    function readUint32(bytes memory b, uint256 index) internal pure returns (uint32) {
        require(b.length >= index + 4, "index out of bounds");
        bytes4 result;
        assembly {
            result := mload(add(b, add(index, 32)))
        }
        return uint32(result);
    }

    function readUint64(bytes memory b, uint256 index) internal pure returns (uint64) {
        require(b.length >= index + 8, "index out of bounds");
        bytes8 result;
        assembly {
            result := mload(add(b, add(index, 32)))
        }
        return uint64(result);
    }

    function memcpy(uint256 dest, uint256 src, uint256 len) internal pure {
        // Copy word-length chunks while possible
        for (; len >= 32; len -= 32) {
            assembly {
                mstore(dest, mload(src))
            }
            dest += 32;
            src += 32;
        }

        if (len > 0) {
            // Copy remaining bytes
            uint256 mask = 256 ** (32 - len) - 1;
            assembly {
                let srcpart := and(mload(src), not(mask))
                let destpart := and(mload(dest), mask)
                mstore(dest, or(destpart, srcpart))
            }
        }
    }
}

// lib/solidity-lib/contracts/libs/utils/MemoryUtils.sol

/**
 * @title MemoryUtils
 * @notice A library that provides utility functions for memory manipulation in Solidity.
 */
library MemoryUtils {
    /**
     * @notice Copies the contents of the source bytes to the destination bytes. strings can be casted
     * to bytes in order to use this function.
     *
     * @param source_ The source bytes to copy from.
     * @return destination_ The newly allocated bytes.
     */
    function copy(bytes memory source_) internal view returns (bytes memory destination_) {
        destination_ = new bytes(source_.length);

        unsafeCopy(getDataPointer(source_), getDataPointer(destination_), source_.length);
    }

    /**
     * @notice Copies the contents of the source bytes32 array to the destination bytes32 array.
     * uint256[], address[] array can be casted to bytes32[] via `TypeCaster` library.
     *
     * @param source_ The source bytes32 array to copy from.
     * @return destination_ The newly allocated bytes32 array.
     */
    function copy(bytes32[] memory source_) internal view returns (bytes32[] memory destination_) {
        destination_ = new bytes32[](source_.length);

        unsafeCopy(getDataPointer(source_), getDataPointer(destination_), source_.length * 32);
    }

    /**
     * @notice Copies memory from one location to another efficiently via identity precompile.
     * @param sourcePointer_ The offset in the memory from which to copy.
     * @param destinationPointer_ The offset in the memory where the result will be copied.
     * @param size_ The size of the memory to copy.
     *
     * @dev This function does not account for free memory pointer and should be used with caution.
     *
     * This signature of calling identity precompile is:
     * staticcall(gas(), address(0x04), argsOffset, argsSize, retOffset, retSize)
     */
    function unsafeCopy(
        uint256 sourcePointer_,
        uint256 destinationPointer_,
        uint256 size_
    ) internal view {
        assembly {
            pop(staticcall(gas(), 4, sourcePointer_, size_, destinationPointer_, size_))
        }
    }

    /**
     * @notice Returns the memory pointer to the given bytes starting position including the length.
     */
    function getPointer(bytes memory data_) internal pure returns (uint256 pointer_) {
        assembly {
            pointer_ := data_
        }
    }

    /**
     * @notice Returns the memory pointer to the given bytes starting position including the length.
     * Cast uint256[] and address[] to bytes32[] via `TypeCaster` library.
     */
    function getPointer(bytes32[] memory data_) internal pure returns (uint256 pointer_) {
        assembly {
            pointer_ := data_
        }
    }

    /**
     * @notice Returns the memory pointer to the given bytes data starting position skipping the length.
     */
    function getDataPointer(bytes memory data_) internal pure returns (uint256 pointer_) {
        assembly {
            pointer_ := add(data_, 32)
        }
    }

    /**
     * @notice Returns the memory pointer to the given bytes data starting position skipping the length.
     * Cast uint256[] and address[] to bytes32[] via `TypeCaster` library.
     */
    function getDataPointer(bytes32[] memory data_) internal pure returns (uint256 pointer_) {
        assembly {
            pointer_ := add(data_, 32)
        }
    }
}

// src/CborDecode.sol

type CborElement is uint256;

library LibCborElement {
    // Cbor element type
    function cborType(CborElement self) internal pure returns (uint8) {
        return uint8(CborElement.unwrap(self));
    }

    // First byte index of the content
    function start(CborElement self) internal pure returns (uint256) {
        return uint80(CborElement.unwrap(self) >> 80);
    }

    // First byte index of the next element (exclusive end of content)
    function end(CborElement self) internal pure returns (uint256) {
        return start(self) + length(self);
    }

    // Content length (0 for non-string types)
    function length(CborElement self) internal pure returns (uint256) {
        uint8 _type = cborType(self);
        if (_type == 0x40 || _type == 0x60) {
            // length is non-zero only for byte strings and text strings
            return value(self);
        }
        return 0;
    }

    // Value of the element (length for string/map/array types, value for others)
    function value(CborElement self) internal pure returns (uint64) {
        return uint64(CborElement.unwrap(self) >> 160);
    }

    // Returns true if the element is null
    function isNull(CborElement self) internal pure returns (bool) {
        uint8 _type = cborType(self);
        return _type == 0xf6 || _type == 0xf7; // null or undefined
    }

    // Pack 3 uint80s into a uint256
    function toCborElement(uint256 _type, uint256 _start, uint256 _length) internal pure returns (CborElement) {
        return CborElement.wrap(_type | _start << 80 | _length << 160);
    }
}

library CborDecode {
    using LibBytes for bytes;
    using LibCborElement for CborElement;

    // Calculate the keccak256 hash of the given cbor element
    function keccak(bytes memory cbor, CborElement ptr) internal pure returns (bytes32) {
        return cbor.keccak(ptr.start(), ptr.length());
    }

    // Take a slice of the given cbor element
    function slice(bytes memory cbor, CborElement ptr) internal pure returns (bytes memory) {
        return cbor.slice(ptr.start(), ptr.length());
    }

    function byteStringAt(bytes memory cbor, uint256 ix) internal pure returns (CborElement) {
        return elementAt(cbor, ix, 0x40, true);
    }

    function nextByteString(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return elementAt(cbor, ptr.end(), 0x40, true);
    }

    function nextByteStringOrNull(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return elementAt(cbor, ptr.end(), 0x40, false);
    }

    function nextTextString(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return elementAt(cbor, ptr.end(), 0x60, true);
    }

    function nextPositiveInt(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return elementAt(cbor, ptr.end(), 0x00, true);
    }

    function mapAt(bytes memory cbor, uint256 ix) internal pure returns (CborElement) {
        return elementAt(cbor, ix, 0xa0, true);
    }

    function nextMap(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return mapAt(cbor, ptr.end());
    }

    function nextArray(bytes memory cbor, CborElement ptr) internal pure returns (CborElement) {
        return elementAt(cbor, ptr.end(), 0x80, true);
    }

    function elementAt(bytes memory cbor, uint256 ix, uint8 expectedType, bool required)
        internal
        pure
        returns (CborElement)
    {
        uint8 _type = uint8(cbor[ix] & 0xe0);
        uint8 ai = uint8(cbor[ix] & 0x1f);
        if (_type == 0xe0) {
            // The primitive type can encode a float, bool, null, undefined, etc.
            // We only need support for null (and we treat undefined as null).
            require(ai == 22 || ai == 23, "only null primitive values are supported");
            require(!required, "null value for required element");
            // retain the additional information:
            return LibCborElement.toCborElement(_type | ai, ix + 1, 0);
        }
        require(_type == expectedType, "unexpected type");
        require(ai < 28, "unsupported type");
        if (ai == 24) {
            return LibCborElement.toCborElement(_type, ix + 2, uint8(cbor[ix + 1]));
        } else if (ai == 25) {
            return LibCborElement.toCborElement(_type, ix + 3, cbor.readUint16(ix + 1));
        } else if (ai == 26) {
            return LibCborElement.toCborElement(_type, ix + 5, cbor.readUint32(ix + 1));
        } else if (ai == 27) {
            return LibCborElement.toCborElement(_type, ix + 9, cbor.readUint64(ix + 1));
        }
        return LibCborElement.toCborElement(_type, ix + 1, ai);
    }
}

// lib/solidity-lib/contracts/libs/crypto/ECDSA384.sol

/**
 * @notice Cryptography module
 *
 * This library provides functionality for ECDSA verification over any 384-bit curve. Currently,
 * this is the most efficient implementation out there, consuming ~8.025 million gas per call.
 *
 * The approach is Strauss-Shamir double scalar multiplication with 6 bits of precompute + affine coordinates.
 * For reference, naive implementation uses ~400 billion gas, which is 50000 times more expensive.
 *
 * We also tried using projective coordinates, however, the gas consumption rose to ~9 million gas.
 */
library ECDSA384 {
    using MemoryUtils for *;
    using U384 for *;

    /**
     * @notice 384-bit curve parameters.
     */
    struct Parameters {
        bytes a;
        bytes b;
        bytes gx;
        bytes gy;
        bytes p;
        bytes n;
        bytes lowSmax;
    }

    struct _Parameters {
        uint256 a;
        uint256 b;
        uint256 gx;
        uint256 gy;
        uint256 p;
        uint256 n;
        uint256 lowSmax;
    }

    struct _Inputs {
        uint256 r;
        uint256 s;
        uint256 x;
        uint256 y;
    }

    /**
     * @notice The function to verify the ECDSA signature
     * @param curveParams_ the 384-bit curve parameters. `lowSmax` is `n / 2`.
     * @param hashedMessage_ the already hashed message to be verified.
     * @param signature_ the ECDSA signature. Equals to `bytes(r) + bytes(s)`.
     * @param pubKey_ the full public key of a signer. Equals to `bytes(x) + bytes(y)`.
     *
     * Note that signatures only from the lower part of the curve are accepted.
     * If your `s > n / 2`, change it to `s = n - s`.
     */
    function verify(
        Parameters memory curveParams_,
        bytes memory hashedMessage_,
        bytes memory signature_,
        bytes memory pubKey_
    ) internal view returns (bool) {
        unchecked {
            _Inputs memory inputs_;

            (inputs_.r, inputs_.s) = U384.init2(signature_);
            (inputs_.x, inputs_.y) = U384.init2(pubKey_);

            _Parameters memory params_ = _Parameters({
                a: curveParams_.a.init(),
                b: curveParams_.b.init(),
                gx: curveParams_.gx.init(),
                gy: curveParams_.gy.init(),
                p: curveParams_.p.init(),
                n: curveParams_.n.init(),
                lowSmax: curveParams_.lowSmax.init()
            });

            uint256 call = U384.initCall(params_.p);

            /// accept s only from the lower part of the curve
            if (
                U384.eqInteger(inputs_.r, 0) ||
                U384.cmp(inputs_.r, params_.n) >= 0 ||
                U384.eqInteger(inputs_.s, 0) ||
                U384.cmp(inputs_.s, params_.lowSmax) > 0
            ) {
                return false;
            }

            if (!_isOnCurve(call, params_.p, params_.a, params_.b, inputs_.x, inputs_.y)) {
                return false;
            }

            /// allow compatibility with non-384-bit hash functions.
            {
                uint256 hashedMessageLength_ = hashedMessage_.length;

                if (hashedMessageLength_ < 48) {
                    bytes memory tmp_ = new bytes(48);

                    MemoryUtils.unsafeCopy(
                        hashedMessage_.getDataPointer(),
                        tmp_.getDataPointer() + 48 - hashedMessageLength_,
                        hashedMessageLength_
                    );

                    hashedMessage_ = tmp_;
                }
            }

            uint256 scalar1 = U384.moddiv(call, hashedMessage_.init(), inputs_.s, params_.n);
            uint256 scalar2 = U384.moddiv(call, inputs_.r, inputs_.s, params_.n);

            {
                uint256 three = U384.init(3);

                /// We use 6-bit masks where the first 3 bits refer to `scalar1` and the last 3 bits refer to `scalar2`.
                uint256[2][64] memory points_ = _precomputePointsTable(
                    call,
                    params_.p,
                    three,
                    params_.a,
                    params_.gx,
                    params_.gy,
                    inputs_.x,
                    inputs_.y
                );

                (scalar1, ) = _doubleScalarMultiplication(
                    call,
                    params_.p,
                    three,
                    params_.a,
                    points_,
                    scalar1,
                    scalar2
                );
            }

            U384.modAssign(call, scalar1, params_.n);

            return U384.eq(scalar1, inputs_.r);
        }
    }

    /**
     * @dev Check if a point in affine coordinates is on the curve.
     */
    function _isOnCurve(
        uint256 call,
        uint256 p,
        uint256 a,
        uint256 b,
        uint256 x,
        uint256 y
    ) private view returns (bool) {
        unchecked {
            if (U384.eqInteger(x, 0) || U384.eq(x, p) || U384.eqInteger(y, 0) || U384.eq(y, p)) {
                return false;
            }

            uint256 LHS = U384.modexp(call, y, 2);
            uint256 RHS = U384.modexp(call, x, 3);

            if (!U384.eqInteger(a, 0)) {
                RHS = U384.modadd(RHS, U384.modmul(call, x, a), p); // x^3 + a*x
            }

            if (!U384.eqInteger(b, 0)) {
                RHS = U384.modadd(RHS, b, p); // x^3 + a*x + b
            }

            return U384.eq(LHS, RHS);
        }
    }

    /**
     * @dev Compute the Strauss-Shamir double scalar multiplication scalar1*G + scalar2*H.
     */
    function _doubleScalarMultiplication(
        uint256 call,
        uint256 p,
        uint256 three,
        uint256 a,
        uint256[2][64] memory points,
        uint256 scalar1,
        uint256 scalar2
    ) private view returns (uint256 x, uint256 y) {
        unchecked {
            uint256 mask_;
            uint256 scalar1Bits_;
            uint256 scalar2Bits_;

            assembly {
                scalar1Bits_ := mload(scalar1)
                scalar2Bits_ := mload(scalar2)
            }

            (x, y) = _twiceAffine(call, p, three, a, x, y);

            mask_ = ((scalar1Bits_ >> 183) << 3) | (scalar2Bits_ >> 183);

            if (mask_ != 0) {
                (x, y) = _addAffine(call, p, three, a, points[mask_][0], points[mask_][1], x, y);
            }

            for (uint256 word = 4; word <= 184; word += 3) {
                (x, y) = _twice3Affine(call, p, three, a, x, y);

                mask_ =
                    (((scalar1Bits_ >> (184 - word)) & 0x07) << 3) |
                    ((scalar2Bits_ >> (184 - word)) & 0x07);

                if (mask_ != 0) {
                    (x, y) = _addAffine(
                        call,
                        p,
                        three,
                        a,
                        points[mask_][0],
                        points[mask_][1],
                        x,
                        y
                    );
                }
            }

            assembly {
                scalar1Bits_ := mload(add(scalar1, 0x20))
                scalar2Bits_ := mload(add(scalar2, 0x20))
            }

            (x, y) = _twiceAffine(call, p, three, a, x, y);

            mask_ = ((scalar1Bits_ >> 255) << 3) | (scalar2Bits_ >> 255);

            if (mask_ != 0) {
                (x, y) = _addAffine(call, p, three, a, points[mask_][0], points[mask_][1], x, y);
            }

            for (uint256 word = 4; word <= 256; word += 3) {
                (x, y) = _twice3Affine(call, p, three, a, x, y);

                mask_ =
                    (((scalar1Bits_ >> (256 - word)) & 0x07) << 3) |
                    ((scalar2Bits_ >> (256 - word)) & 0x07);

                if (mask_ != 0) {
                    (x, y) = _addAffine(
                        call,
                        p,
                        three,
                        a,
                        points[mask_][0],
                        points[mask_][1],
                        x,
                        y
                    );
                }
            }
        }
    }

    /**
     * @dev Double an elliptic curve point in affine coordinates.
     */
    function _twiceAffine(
        uint256 call,
        uint256 p,
        uint256 three,
        uint256 a,
        uint256 x1,
        uint256 y1
    ) private view returns (uint256 x2, uint256 y2) {
        unchecked {
            if (x1 == 0) {
                return (0, 0);
            }

            if (U384.eqInteger(y1, 0)) {
                return (0, 0);
            }

            uint256 m1 = U384.modexp(call, x1, 2);
            U384.modmulAssign(call, m1, three);
            U384.modaddAssign(m1, a, p);

            uint256 m2 = U384.modshl1(y1, p);
            U384.moddivAssign(call, m1, m2);

            x2 = U384.modexp(call, m1, 2);
            U384.modsubAssign(x2, x1, p);
            U384.modsubAssign(x2, x1, p);

            y2 = U384.modsub(x1, x2, p);
            U384.modmulAssign(call, y2, m1);
            U384.modsubAssign(y2, y1, p);
        }
    }

    /**
     * @dev Doubles an elliptic curve point 3 times in affine coordinates.
     */
    function _twice3Affine(
        uint256 call,
        uint256 p,
        uint256 three,
        uint256 a,
        uint256 x1,
        uint256 y1
    ) private view returns (uint256 x2, uint256 y2) {
        unchecked {
            if (x1 == 0) {
                return (0, 0);
            }

            if (U384.eqInteger(y1, 0)) {
                return (0, 0);
            }

            uint256 m1 = U384.modexp(call, x1, 2);
            U384.modmulAssign(call, m1, three);
            U384.modaddAssign(m1, a, p);

            uint256 m2 = U384.modshl1(y1, p);
            U384.moddivAssign(call, m1, m2);

            x2 = U384.modexp(call, m1, 2);
            U384.modsubAssign(x2, x1, p);
            U384.modsubAssign(x2, x1, p);

            y2 = U384.modsub(x1, x2, p);
            U384.modmulAssign(call, y2, m1);
            U384.modsubAssign(y2, y1, p);

            if (U384.eqInteger(y2, 0)) {
                return (0, 0);
            }

            U384.modexpAssignTo(call, m1, x2, 2);
            U384.modmulAssign(call, m1, three);
            U384.modaddAssign(m1, a, p);

            U384.modshl1AssignTo(m2, y2, p);
            U384.moddivAssign(call, m1, m2);

            U384.modexpAssignTo(call, x1, m1, 2);
            U384.modsubAssign(x1, x2, p);
            U384.modsubAssign(x1, x2, p);

            U384.modsubAssignTo(y1, x2, x1, p);
            U384.modmulAssign(call, y1, m1);
            U384.modsubAssign(y1, y2, p);

            if (U384.eqInteger(y1, 0)) {
                return (0, 0);
            }

            U384.modexpAssignTo(call, m1, x1, 2);
            U384.modmulAssign(call, m1, three);
            U384.modaddAssign(m1, a, p);

            U384.modshl1AssignTo(m2, y1, p);
            U384.moddivAssign(call, m1, m2);

            U384.modexpAssignTo(call, x2, m1, 2);
            U384.modsubAssign(x2, x1, p);
            U384.modsubAssign(x2, x1, p);

            U384.modsubAssignTo(y2, x1, x2, p);
            U384.modmulAssign(call, y2, m1);
            U384.modsubAssign(y2, y1, p);
        }
    }

    /**
     * @dev Add two elliptic curve points in affine coordinates.
     */
    function _addAffine(
        uint256 call,
        uint256 p,
        uint256 three,
        uint256 a,
        uint256 x1,
        uint256 y1,
        uint256 x2,
        uint256 y2
    ) private view returns (uint256 x3, uint256 y3) {
        unchecked {
            if (x1 == 0 || x2 == 0) {
                if (x1 == 0 && x2 == 0) {
                    return (0, 0);
                }

                return x1 == 0 ? (x2.copy(), y2.copy()) : (x1.copy(), y1.copy());
            }

            if (U384.eq(x1, x2)) {
                if (U384.eq(y1, y2)) {
                    return _twiceAffine(call, p, three, a, x1, y1);
                }

                return (0, 0);
            }

            uint256 m1 = U384.modsub(y1, y2, p);
            uint256 m2 = U384.modsub(x1, x2, p);

            U384.moddivAssign(call, m1, m2);

            x3 = U384.modexp(call, m1, 2);
            U384.modsubAssign(x3, x1, p);
            U384.modsubAssign(x3, x2, p);

            y3 = U384.modsub(x1, x3, p);
            U384.modmulAssign(call, y3, m1);
            U384.modsubAssign(y3, y1, p);
        }
    }

    function _precomputePointsTable(
        uint256 call,
        uint256 p,
        uint256 three,
        uint256 a,
        uint256 gx,
        uint256 gy,
        uint256 hx,
        uint256 hy
    ) private view returns (uint256[2][64] memory points_) {
        unchecked {
            (points_[0x01][0], points_[0x01][1]) = (hx.copy(), hy.copy());
            (points_[0x08][0], points_[0x08][1]) = (gx.copy(), gy.copy());

            for (uint256 i = 0; i < 8; ++i) {
                for (uint256 j = 0; j < 8; ++j) {
                    if (i + j < 2) {
                        continue;
                    }

                    uint256 maskTo = (i << 3) | j;

                    if (i != 0) {
                        uint256 maskFrom = ((i - 1) << 3) | j;

                        (points_[maskTo][0], points_[maskTo][1]) = _addAffine(
                            call,
                            p,
                            three,
                            a,
                            points_[maskFrom][0],
                            points_[maskFrom][1],
                            gx,
                            gy
                        );
                    } else {
                        uint256 maskFrom = (i << 3) | (j - 1);

                        (points_[maskTo][0], points_[maskTo][1]) = _addAffine(
                            call,
                            p,
                            three,
                            a,
                            points_[maskFrom][0],
                            points_[maskFrom][1],
                            hx,
                            hy
                        );
                    }
                }
            }
        }
    }
}

/**
 * @notice Low-level utility library that implements unsigned 384-bit arithmetics.
 *
 * Should not be used outside of this file.
 */
library U384 {
    uint256 private constant SHORT_ALLOCATION = 64;

    uint256 private constant CALL_ALLOCATION = 4 * 288;

    uint256 private constant MUL_OFFSET = 288;
    uint256 private constant EXP_OFFSET = 2 * 288;
    uint256 private constant INV_OFFSET = 3 * 288;

    function init(uint256 from_) internal pure returns (uint256 handler_) {
        unchecked {
            handler_ = _allocate(SHORT_ALLOCATION);

            assembly {
                mstore(handler_, 0x00)
                mstore(add(0x20, handler_), from_)
            }

            return handler_;
        }
    }

    function init(bytes memory from_) internal pure returns (uint256 handler_) {
        unchecked {
            require(from_.length == 48, "U384: not 384");

            handler_ = _allocate(SHORT_ALLOCATION);

            assembly {
                mstore(handler_, 0x00)
                mstore(add(handler_, 0x10), mload(add(from_, 0x20)))
                mstore(add(handler_, 0x20), mload(add(from_, 0x30)))
            }

            return handler_;
        }
    }

    function init2(
        bytes memory from2_
    ) internal pure returns (uint256 handler1_, uint256 handler2_) {
        unchecked {
            require(from2_.length == 96, "U384: not 768");

            handler1_ = _allocate(SHORT_ALLOCATION);
            handler2_ = _allocate(SHORT_ALLOCATION);

            assembly {
                mstore(handler1_, 0x00)
                mstore(add(handler1_, 0x10), mload(add(from2_, 0x20)))
                mstore(add(handler1_, 0x20), mload(add(from2_, 0x30)))

                mstore(handler2_, 0x00)
                mstore(add(handler2_, 0x10), mload(add(from2_, 0x50)))
                mstore(add(handler2_, 0x20), mload(add(from2_, 0x60)))
            }

            return (handler1_, handler2_);
        }
    }

    function initCall(uint256 m_) internal pure returns (uint256 handler_) {
        unchecked {
            handler_ = _allocate(CALL_ALLOCATION);

            _sub(m_, init(2), handler_ + INV_OFFSET + 0xA0);

            assembly {
                let call_ := add(handler_, MUL_OFFSET)

                mstore(call_, 0x60)
                mstore(add(0x20, call_), 0x20)
                mstore(add(0x40, call_), 0x40)
                mstore(add(0xC0, call_), 0x01)
                mstore(add(0xE0, call_), mload(m_))
                mstore(add(0x0100, call_), mload(add(m_, 0x20)))

                call_ := add(handler_, EXP_OFFSET)

                mstore(call_, 0x40)
                mstore(add(0x20, call_), 0x20)
                mstore(add(0x40, call_), 0x40)
                mstore(add(0xC0, call_), mload(m_))
                mstore(add(0xE0, call_), mload(add(m_, 0x20)))

                call_ := add(handler_, INV_OFFSET)

                mstore(call_, 0x40)
                mstore(add(0x20, call_), 0x40)
                mstore(add(0x40, call_), 0x40)
                mstore(add(0xE0, call_), mload(m_))
                mstore(add(0x0100, call_), mload(add(m_, 0x20)))
            }
        }
    }

    function copy(uint256 handler_) internal pure returns (uint256 handlerCopy_) {
        unchecked {
            handlerCopy_ = _allocate(SHORT_ALLOCATION);

            assembly {
                mstore(handlerCopy_, mload(handler_))
                mstore(add(handlerCopy_, 0x20), mload(add(handler_, 0x20)))
            }

            return handlerCopy_;
        }
    }

    function eq(uint256 a_, uint256 b_) internal pure returns (bool eq_) {
        assembly {
            eq_ := and(eq(mload(a_), mload(b_)), eq(mload(add(a_, 0x20)), mload(add(b_, 0x20))))
        }
    }

    function eqInteger(uint256 a_, uint256 bInteger_) internal pure returns (bool eq_) {
        assembly {
            eq_ := and(eq(mload(a_), 0), eq(mload(add(a_, 0x20)), bInteger_))
        }
    }

    function cmp(uint256 a_, uint256 b_) internal pure returns (int256 cmp_) {
        unchecked {
            uint256 aWord_;
            uint256 bWord_;

            assembly {
                aWord_ := mload(a_)
                bWord_ := mload(b_)
            }

            if (aWord_ > bWord_) {
                return 1;
            }

            if (aWord_ < bWord_) {
                return -1;
            }

            assembly {
                aWord_ := mload(add(a_, 0x20))
                bWord_ := mload(add(b_, 0x20))
            }

            if (aWord_ > bWord_) {
                return 1;
            }

            if (aWord_ < bWord_) {
                return -1;
            }
        }
    }

    function modAssign(uint256 call_, uint256 a_, uint256 m_) internal view {
        assembly {
            mstore(call_, 0x40)
            mstore(add(0x20, call_), 0x20)
            mstore(add(0x40, call_), 0x40)
            mstore(add(0x60, call_), mload(a_))
            mstore(add(0x80, call_), mload(add(a_, 0x20)))
            mstore(add(0xA0, call_), 0x01)
            mstore(add(0xC0, call_), mload(m_))
            mstore(add(0xE0, call_), mload(add(m_, 0x20)))

            pop(staticcall(gas(), 0x5, call_, 0x0100, a_, 0x40))
        }
    }

    function modexp(
        uint256 call_,
        uint256 b_,
        uint256 eInteger_
    ) internal view returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            assembly {
                call_ := add(call_, EXP_OFFSET)

                mstore(add(0x60, call_), mload(b_))
                mstore(add(0x80, call_), mload(add(b_, 0x20)))
                mstore(add(0xA0, call_), eInteger_)

                pop(staticcall(gas(), 0x5, call_, 0x0100, r_, 0x40))
            }

            return r_;
        }
    }

    function modexpAssignTo(
        uint256 call_,
        uint256 to_,
        uint256 b_,
        uint256 eInteger_
    ) internal view {
        assembly {
            call_ := add(call_, EXP_OFFSET)

            mstore(add(0x60, call_), mload(b_))
            mstore(add(0x80, call_), mload(add(b_, 0x20)))
            mstore(add(0xA0, call_), eInteger_)

            pop(staticcall(gas(), 0x5, call_, 0x0100, to_, 0x40))
        }
    }

    function modadd(uint256 a_, uint256 b_, uint256 m_) internal pure returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            _add(a_, b_, r_);

            if (cmp(r_, m_) >= 0) {
                _subFrom(r_, m_);
            }

            return r_;
        }
    }

    function modaddAssign(uint256 a_, uint256 b_, uint256 m_) internal pure {
        unchecked {
            _addTo(a_, b_);

            if (cmp(a_, m_) >= 0) {
                return _subFrom(a_, m_);
            }
        }
    }

    function modmul(uint256 call_, uint256 a_, uint256 b_) internal view returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            _mul(a_, b_, call_ + MUL_OFFSET + 0x60);

            assembly {
                call_ := add(call_, MUL_OFFSET)

                pop(staticcall(gas(), 0x5, call_, 0x0120, r_, 0x40))
            }

            return r_;
        }
    }

    function modmulAssign(uint256 call_, uint256 a_, uint256 b_) internal view {
        unchecked {
            _mul(a_, b_, call_ + MUL_OFFSET + 0x60);

            assembly {
                call_ := add(call_, MUL_OFFSET)

                pop(staticcall(gas(), 0x5, call_, 0x0120, a_, 0x40))
            }
        }
    }

    function modsub(uint256 a_, uint256 b_, uint256 m_) internal pure returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            if (cmp(a_, b_) >= 0) {
                _sub(a_, b_, r_);
                return r_;
            }

            _add(a_, m_, r_);
            _subFrom(r_, b_);
        }
    }

    function modsubAssign(uint256 a_, uint256 b_, uint256 m_) internal pure {
        unchecked {
            if (cmp(a_, b_) >= 0) {
                _subFrom(a_, b_);
                return;
            }

            _addTo(a_, m_);
            _subFrom(a_, b_);
        }
    }

    function modsubAssignTo(uint256 to_, uint256 a_, uint256 b_, uint256 m_) internal pure {
        unchecked {
            if (cmp(a_, b_) >= 0) {
                _sub(a_, b_, to_);
                return;
            }

            _add(a_, m_, to_);
            _subFrom(to_, b_);
        }
    }

    function modshl1(uint256 a_, uint256 m_) internal pure returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            _shl1(a_, r_);

            if (cmp(r_, m_) >= 0) {
                _subFrom(r_, m_);
            }

            return r_;
        }
    }

    function modshl1AssignTo(uint256 to_, uint256 a_, uint256 m_) internal pure {
        unchecked {
            _shl1(a_, to_);

            if (cmp(to_, m_) >= 0) {
                _subFrom(to_, m_);
            }
        }
    }

    /// @dev Stores modinv into `b_` and moddiv into `a_`.
    function moddivAssign(uint256 call_, uint256 a_, uint256 b_) internal view {
        unchecked {
            assembly {
                call_ := add(call_, INV_OFFSET)

                mstore(add(0x60, call_), mload(b_))
                mstore(add(0x80, call_), mload(add(b_, 0x20)))

                pop(staticcall(gas(), 0x5, call_, 0x0120, b_, 0x40))
            }

            modmulAssign(call_ - INV_OFFSET, a_, b_);
        }
    }

    function moddiv(
        uint256 call_,
        uint256 a_,
        uint256 b_,
        uint256 m_
    ) internal view returns (uint256 r_) {
        unchecked {
            r_ = modinv(call_, b_, m_);

            _mul(a_, r_, call_ + 0x60);

            assembly {
                mstore(call_, 0x60)
                mstore(add(0x20, call_), 0x20)
                mstore(add(0x40, call_), 0x40)
                mstore(add(0xC0, call_), 0x01)
                mstore(add(0xE0, call_), mload(m_))
                mstore(add(0x0100, call_), mload(add(m_, 0x20)))

                pop(staticcall(gas(), 0x5, call_, 0x0120, r_, 0x40))
            }
        }
    }

    function modinv(uint256 call_, uint256 b_, uint256 m_) internal view returns (uint256 r_) {
        unchecked {
            r_ = _allocate(SHORT_ALLOCATION);

            _sub(m_, init(2), call_ + 0xA0);

            assembly {
                mstore(call_, 0x40)
                mstore(add(0x20, call_), 0x40)
                mstore(add(0x40, call_), 0x40)
                mstore(add(0x60, call_), mload(b_))
                mstore(add(0x80, call_), mload(add(b_, 0x20)))
                mstore(add(0xE0, call_), mload(m_))
                mstore(add(0x0100, call_), mload(add(m_, 0x20)))

                pop(staticcall(gas(), 0x5, call_, 0x0120, r_, 0x40))
            }
        }
    }

    function _shl1(uint256 a_, uint256 r_) internal pure {
        assembly {
            let a1_ := mload(add(a_, 0x20))

            mstore(r_, or(shl(1, mload(a_)), shr(255, a1_)))
            mstore(add(r_, 0x20), shl(1, a1_))
        }
    }

    function _add(uint256 a_, uint256 b_, uint256 r_) private pure {
        assembly {
            let aWord_ := mload(add(a_, 0x20))
            let sum_ := add(aWord_, mload(add(b_, 0x20)))

            mstore(add(r_, 0x20), sum_)

            sum_ := gt(aWord_, sum_)
            sum_ := add(sum_, add(mload(a_), mload(b_)))

            mstore(r_, sum_)
        }
    }

    function _sub(uint256 a_, uint256 b_, uint256 r_) private pure {
        assembly {
            let aWord_ := mload(add(a_, 0x20))
            let diff_ := sub(aWord_, mload(add(b_, 0x20)))

            mstore(add(r_, 0x20), diff_)

            diff_ := gt(diff_, aWord_)
            diff_ := sub(sub(mload(a_), mload(b_)), diff_)

            mstore(r_, diff_)
        }
    }

    function _subFrom(uint256 a_, uint256 b_) private pure {
        assembly {
            let aWord_ := mload(add(a_, 0x20))
            let diff_ := sub(aWord_, mload(add(b_, 0x20)))

            mstore(add(a_, 0x20), diff_)

            diff_ := gt(diff_, aWord_)
            diff_ := sub(sub(mload(a_), mload(b_)), diff_)

            mstore(a_, diff_)
        }
    }

    function _addTo(uint256 a_, uint256 b_) private pure {
        assembly {
            let aWord_ := mload(add(a_, 0x20))
            let sum_ := add(aWord_, mload(add(b_, 0x20)))

            mstore(add(a_, 0x20), sum_)

            sum_ := gt(aWord_, sum_)
            sum_ := add(sum_, add(mload(a_), mload(b_)))

            mstore(a_, sum_)
        }
    }

    function _mul(uint256 a_, uint256 b_, uint256 r_) private pure {
        assembly {
            let a0_ := mload(a_)
            let a1_ := shr(128, mload(add(a_, 0x20)))
            let a2_ := and(mload(add(a_, 0x20)), 0xffffffffffffffffffffffffffffffff)

            let b0_ := mload(b_)
            let b1_ := shr(128, mload(add(b_, 0x20)))
            let b2_ := and(mload(add(b_, 0x20)), 0xffffffffffffffffffffffffffffffff)

            // r5
            let current_ := mul(a2_, b2_)
            let r0_ := and(current_, 0xffffffffffffffffffffffffffffffff)

            // r4
            current_ := shr(128, current_)

            let temp_ := mul(a1_, b2_)
            current_ := add(current_, temp_)
            let curry_ := lt(current_, temp_)

            temp_ := mul(a2_, b1_)
            current_ := add(current_, temp_)
            curry_ := add(curry_, lt(current_, temp_))

            mstore(add(r_, 0x40), add(shl(128, current_), r0_))

            // r3
            current_ := add(shl(128, curry_), shr(128, current_))
            curry_ := 0

            temp_ := mul(a0_, b2_)
            current_ := add(current_, temp_)
            curry_ := lt(current_, temp_)

            temp_ := mul(a1_, b1_)
            current_ := add(current_, temp_)
            curry_ := add(curry_, lt(current_, temp_))

            temp_ := mul(a2_, b0_)
            current_ := add(current_, temp_)
            curry_ := add(curry_, lt(current_, temp_))

            r0_ := and(current_, 0xffffffffffffffffffffffffffffffff)

            // r2
            current_ := add(shl(128, curry_), shr(128, current_))
            curry_ := 0

            temp_ := mul(a0_, b1_)
            current_ := add(current_, temp_)
            curry_ := lt(current_, temp_)

            temp_ := mul(a1_, b0_)
            current_ := add(current_, temp_)
            curry_ := add(curry_, lt(current_, temp_))

            mstore(add(r_, 0x20), add(shl(128, current_), r0_))

            // r1
            current_ := add(shl(128, curry_), shr(128, current_))
            current_ := add(current_, mul(a0_, b0_))

            mstore(r_, current_)
        }
    }

    function _allocate(uint256 bytes_) private pure returns (uint256 handler_) {
        unchecked {
            assembly {
                handler_ := mload(0x40)
                mstore(0x40, add(handler_, bytes_))
            }

            return handler_;
        }
    }
}

// src/Sha2Ext.sol

// adapted from https://github.com/yangfh2004/SolSha2Ext/blob/main/contracts/lib/Sha2Ext.sol

library Sha2Ext {
    using LibBytes for bytes;

    function sha2(bytes memory message, uint256 offset, uint256 length, uint64[8] memory h) internal pure {
        uint64[80] memory k = [
            0x428a2f98d728ae22,
            0x7137449123ef65cd,
            0xb5c0fbcfec4d3b2f,
            0xe9b5dba58189dbbc,
            0x3956c25bf348b538,
            0x59f111f1b605d019,
            0x923f82a4af194f9b,
            0xab1c5ed5da6d8118,
            0xd807aa98a3030242,
            0x12835b0145706fbe,
            0x243185be4ee4b28c,
            0x550c7dc3d5ffb4e2,
            0x72be5d74f27b896f,
            0x80deb1fe3b1696b1,
            0x9bdc06a725c71235,
            0xc19bf174cf692694,
            0xe49b69c19ef14ad2,
            0xefbe4786384f25e3,
            0x0fc19dc68b8cd5b5,
            0x240ca1cc77ac9c65,
            0x2de92c6f592b0275,
            0x4a7484aa6ea6e483,
            0x5cb0a9dcbd41fbd4,
            0x76f988da831153b5,
            0x983e5152ee66dfab,
            0xa831c66d2db43210,
            0xb00327c898fb213f,
            0xbf597fc7beef0ee4,
            0xc6e00bf33da88fc2,
            0xd5a79147930aa725,
            0x06ca6351e003826f,
            0x142929670a0e6e70,
            0x27b70a8546d22ffc,
            0x2e1b21385c26c926,
            0x4d2c6dfc5ac42aed,
            0x53380d139d95b3df,
            0x650a73548baf63de,
            0x766a0abb3c77b2a8,
            0x81c2c92e47edaee6,
            0x92722c851482353b,
            0xa2bfe8a14cf10364,
            0xa81a664bbc423001,
            0xc24b8b70d0f89791,
            0xc76c51a30654be30,
            0xd192e819d6ef5218,
            0xd69906245565a910,
            0xf40e35855771202a,
            0x106aa07032bbd1b8,
            0x19a4c116b8d2d0c8,
            0x1e376c085141ab53,
            0x2748774cdf8eeb99,
            0x34b0bcb5e19b48a8,
            0x391c0cb3c5c95a63,
            0x4ed8aa4ae3418acb,
            0x5b9cca4f7763e373,
            0x682e6ff3d6b2b8a3,
            0x748f82ee5defb2fc,
            0x78a5636f43172f60,
            0x84c87814a1f0ab72,
            0x8cc702081a6439ec,
            0x90befffa23631e28,
            0xa4506cebde82bde9,
            0xbef9a3f7b2c67915,
            0xc67178f2e372532b,
            0xca273eceea26619c,
            0xd186b8c721c0c207,
            0xeada7dd6cde0eb1e,
            0xf57d4f7fee6ed178,
            0x06f067aa72176fba,
            0x0a637dc5a2c898a6,
            0x113f9804bef90dae,
            0x1b710b35131c471b,
            0x28db77f523047d84,
            0x32caab7b40c72493,
            0x3c9ebe0a15c9bebc,
            0x431d67c49c100d4c,
            0x4cc5d4becb3e42b6,
            0x597f299cfc657e2a,
            0x5fcb6fab3ad6faec,
            0x6c44198c4a475817
        ];

        require(offset + length <= message.length, "OUT_OF_BOUNDS");
        bytes memory padding = padMessage(message, offset, length);
        require(padding.length % 128 == 0, "PADDING_ERROR");
        uint64[80] memory w;
        uint64[8] memory temp;
        uint64[16] memory blocks;
        uint256 messageLength = (length / 128) * 128;
        unchecked {
            for (uint256 i = 0; i < (messageLength + padding.length); i += 128) {
                if (i < messageLength) {
                    getBlock(message, blocks, offset + i);
                } else {
                    getBlock(padding, blocks, i - messageLength);
                }
                for (uint256 j = 0; j < 16; ++j) {
                    w[j] = blocks[j];
                }
                for (uint256 j = 16; j < 80; ++j) {
                    w[j] = gamma1(w[j - 2]) + w[j - 7] + gamma0(w[j - 15]) + w[j - 16];
                }
                for (uint256 j = 0; j < 8; ++j) {
                    temp[j] = h[j];
                }
                for (uint256 j = 0; j < 80; ++j) {
                    uint64 t1 = temp[7] + sigma1(temp[4]) + ch(temp[4], temp[5], temp[6]) + k[j] + w[j];
                    uint64 t2 = sigma0(temp[0]) + maj(temp[0], temp[1], temp[2]);
                    temp[7] = temp[6];
                    temp[6] = temp[5];
                    temp[5] = temp[4];
                    temp[4] = temp[3] + t1;
                    temp[3] = temp[2];
                    temp[2] = temp[1];
                    temp[1] = temp[0];
                    temp[0] = t1 + t2;
                }
                for (uint256 j = 0; j < 8; ++j) {
                    h[j] += temp[j];
                }
            }
        }
    }

    function sha384(bytes memory message, uint256 offset, uint256 length) internal pure returns (bytes memory) {
        uint64[8] memory h = [
            0xcbbb9d5dc1059ed8,
            0x629a292a367cd507,
            0x9159015a3070dd17,
            0x152fecd8f70e5939,
            0x67332667ffc00b31,
            0x8eb44a8768581511,
            0xdb0c2e0d64f98fa7,
            0x47b5481dbefa4fa4
        ];
        sha2(message, offset, length, h);
        return abi.encodePacked(bytes8(h[0]), bytes8(h[1]), bytes8(h[2]), bytes8(h[3]), bytes8(h[4]), bytes8(h[5]));
    }

    function sha512(bytes memory message, uint256 offset, uint256 length) internal pure returns (bytes memory) {
        uint64[8] memory h = [
            0x6a09e667f3bcc908,
            0xbb67ae8584caa73b,
            0x3c6ef372fe94f82b,
            0xa54ff53a5f1d36f1,
            0x510e527fade682d1,
            0x9b05688c2b3e6c1f,
            0x1f83d9abfb41bd6b,
            0x5be0cd19137e2179
        ];
        sha2(message, offset, length, h);
        return abi.encodePacked(
            bytes8(h[0]),
            bytes8(h[1]),
            bytes8(h[2]),
            bytes8(h[3]),
            bytes8(h[4]),
            bytes8(h[5]),
            bytes8(h[6]),
            bytes8(h[7])
        );
    }

    function padMessage(bytes memory message, uint256 offset, uint256 length) internal pure returns (bytes memory) {
        bytes8 bitLength = bytes8(uint64(length * 8));
        uint256 mdi = length % 128;
        uint256 paddingLength;
        if (mdi < 112) {
            paddingLength = 119 - mdi;
        } else {
            paddingLength = 247 - mdi;
        }
        bytes memory padding = new bytes(paddingLength);
        bytes memory tail = message.slice(offset + length - mdi, mdi);
        return abi.encodePacked(tail, bytes1(0x80), padding, bitLength);
    }

    function getBlock(bytes memory message, uint64[16] memory blocks, uint256 index) internal pure {
        for (uint256 i = 0; i < 16; ++i) {
            blocks[i] = message.readUint64(index + i * 8);
        }
    }

    function ch(uint64 x, uint64 y, uint64 z) internal pure returns (uint64) {
        return (x & y) ^ (~x & z);
    }

    function maj(uint64 x, uint64 y, uint64 z) internal pure returns (uint64) {
        return (x & y) ^ (x & z) ^ (y & z);
    }

    function sigma0(uint64 x) internal pure returns (uint64) {
        return (rotateRight(x, 28) ^ rotateRight(x, 34) ^ rotateRight(x, 39));
    }

    function sigma1(uint64 x) internal pure returns (uint64) {
        return (rotateRight(x, 14) ^ rotateRight(x, 18) ^ rotateRight(x, 41));
    }

    function gamma0(uint64 x) internal pure returns (uint64) {
        return (rotateRight(x, 1) ^ rotateRight(x, 8) ^ (x >> 7));
    }

    function gamma1(uint64 x) internal pure returns (uint64) {
        return (rotateRight(x, 19) ^ rotateRight(x, 61) ^ (x >> 6));
    }

    function rotateRight(uint64 x, uint64 n) internal pure returns (uint64) {
        return (x << (64 - n)) | (x >> n);
    }
}

// src/ECDSA384Curve.sol

library ECDSA384Curve {
    // ECDSA384 curve parameters (NIST P-384)
    bytes public constant CURVE_A =
        hex"fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000fffffffc";
    bytes public constant CURVE_B =
        hex"b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef";
    bytes public constant CURVE_GX =
        hex"aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7";
    bytes public constant CURVE_GY =
        hex"3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f";
    bytes public constant CURVE_P =
        hex"fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff";
    bytes public constant CURVE_N =
        hex"ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973";
    // use n-1 for lowSmax, which allows s-values above n/2
    bytes public constant CURVE_LOW_S_MAX =
        hex"ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52972";

    function p384() internal pure returns (ECDSA384.Parameters memory) {
        return ECDSA384.Parameters({
            a: CURVE_A,
            b: CURVE_B,
            gx: CURVE_GX,
            gy: CURVE_GY,
            p: CURVE_P,
            n: CURVE_N,
            lowSmax: CURVE_LOW_S_MAX
        });
    }
}

// src/NitroValidator.sol

// adapted from https://github.com/marlinprotocol/NitroProver/blob/f1d368d1f172ad3a55cd2aaaa98ad6a6e7dcde9d/src/NitroProver.sol

contract NitroValidator {
    using LibBytes for bytes;
    using CborDecode for bytes;
    using LibCborElement for CborElement;

    bytes32 public constant ATTESTATION_TBS_PREFIX = 0x63ce814bd924c1ef12c43686e4cbf48ed1639a78387b0570c23ca921e8ce071c; // keccak256(hex"846a5369676e61747572653144a101382240")
    bytes32 public constant ATTESTATION_DIGEST = 0x501a3a7a4e0cf54b03f2488098bdd59bc1c2e8d741a300d6b25926d531733fef; // keccak256("SHA384")

    bytes32 public constant CERTIFICATE_KEY = 0x925cec779426f44d8d555e01d2683a3a765ce2fa7562ca7352aeb09dfc57ea6a; // keccak256(bytes("certificate"))
    bytes32 public constant PUBLIC_KEY_KEY = 0xc7b28019ccfdbd30ffc65951d94bb85c9e2b8434111a000b5afd533ce65f57a4; // keccak256(bytes("public_key"))
    bytes32 public constant MODULE_ID_KEY = 0x8ce577cf664c36ba5130242bf5790c2675e9f4e6986a842b607821bee25372ee; // keccak256(bytes("module_id"))
    bytes32 public constant TIMESTAMP_KEY = 0x4ebf727c48eac2c66272456b06a885c5cc03e54d140f63b63b6fd10c1227958e; // keccak256(bytes("timestamp"))
    bytes32 public constant USER_DATA_KEY = 0x5e4ea5393e4327b3014bc32f2264336b0d1ee84a4cfd197c8ad7e1e16829a16a; // keccak256(bytes("user_data"))
    bytes32 public constant CABUNDLE_KEY = 0x8a8cb7aa1da17ada103546ae6b4e13ccc2fafa17adf5f93925e0a0a4e5681a6a; // keccak256(bytes("cabundle"))
    bytes32 public constant DIGEST_KEY = 0x682a7e258d80bd2421d3103cbe71e3e3b82138116756b97b8256f061dc2f11fb; // keccak256(bytes("digest"))
    bytes32 public constant NONCE_KEY = 0x7ab1577440dd7bedf920cb6de2f9fc6bf7ba98c78c85a3fa1f8311aac95e1759; // keccak256(bytes("nonce"))
    bytes32 public constant PCRS_KEY = 0x61585f8bc67a4b6d5891a4639a074964ac66fc2241dc0b36c157dc101325367a; // keccak256(bytes("pcrs"))

    struct Ptrs {
        CborElement moduleID;
        uint64 timestamp;
        CborElement digest;
        CborElement[] pcrs;
        CborElement cert;
        CborElement[] cabundle;
        CborElement publicKey;
        CborElement userData;
        CborElement nonce;
    }

    ICertManager public immutable certManager;

    constructor(ICertManager _certManager) {
        certManager = _certManager;
    }

    function decodeAttestationTbs(bytes memory attestation)
        external
        pure
        returns (bytes memory attestationTbs, bytes memory signature)
    {
        uint256 offset = 1;
        if (attestation[0] == 0xD2) {
            offset = 2;
        }

        CborElement protectedPtr = attestation.byteStringAt(offset);
        CborElement unprotectedPtr = attestation.nextMap(protectedPtr);
        CborElement payloadPtr = attestation.nextByteString(unprotectedPtr);
        CborElement signaturePtr = attestation.nextByteString(payloadPtr);

        uint256 rawProtectedLength = protectedPtr.end() - offset;
        uint256 rawPayloadLength = payloadPtr.end() - unprotectedPtr.end();
        bytes memory rawProtectedBytes = attestation.slice(offset, rawProtectedLength);
        bytes memory rawPayloadBytes = attestation.slice(unprotectedPtr.end(), rawPayloadLength);
        attestationTbs =
            _constructAttestationTbs(rawProtectedBytes, rawProtectedLength, rawPayloadBytes, rawPayloadLength);
        signature = attestation.slice(signaturePtr.start(), signaturePtr.length());
    }

    function validateAttestation(bytes memory attestationTbs, bytes memory signature) public returns (Ptrs memory) {
        Ptrs memory ptrs = _parseAttestation(attestationTbs);

        require(ptrs.moduleID.length() > 0, "no module id");
        require(ptrs.timestamp > 0, "no timestamp");
        require(ptrs.cabundle.length > 0, "no cabundle");
        require(attestationTbs.keccak(ptrs.digest) == ATTESTATION_DIGEST, "invalid digest");
        require(1 <= ptrs.pcrs.length && ptrs.pcrs.length <= 32, "invalid pcrs");
        require(
            ptrs.publicKey.isNull() || (1 <= ptrs.publicKey.length() && ptrs.publicKey.length() <= 1024),
            "invalid pub key"
        );
        require(ptrs.userData.isNull() || (ptrs.userData.length() <= 512), "invalid user data");
        require(ptrs.nonce.isNull() || (ptrs.nonce.length() <= 512), "invalid nonce");

        for (uint256 i = 0; i < ptrs.pcrs.length; i++) {
            require(
                ptrs.pcrs[i].length() == 32 || ptrs.pcrs[i].length() == 48 || ptrs.pcrs[i].length() == 64, "invalid pcr"
            );
        }

        bytes memory cert = attestationTbs.slice(ptrs.cert);
        bytes[] memory cabundle = new bytes[](ptrs.cabundle.length);
        for (uint256 i = 0; i < ptrs.cabundle.length; i++) {
            require(1 <= ptrs.cabundle[i].length() && ptrs.cabundle[i].length() <= 1024, "invalid cabundle cert");
            cabundle[i] = attestationTbs.slice(ptrs.cabundle[i]);
        }

        ICertManager.VerifiedCert memory parent = verifyCertBundle(cert, cabundle);
        bytes memory hash = Sha2Ext.sha384(attestationTbs, 0, attestationTbs.length);
        _verifySignature(parent.pubKey, hash, signature);

        return ptrs;
    }

    function verifyCertBundle(bytes memory certificate, bytes[] memory cabundle)
        internal
        returns (ICertManager.VerifiedCert memory)
    {
        bytes32 parentHash;
        for (uint256 i = 0; i < cabundle.length; i++) {
            parentHash = certManager.verifyCACert(cabundle[i], parentHash);
        }
        return certManager.verifyClientCert(certificate, parentHash);
    }

    function _constructAttestationTbs(
        bytes memory rawProtectedBytes,
        uint256 rawProtectedLength,
        bytes memory rawPayloadBytes,
        uint256 rawPayloadLength
    ) internal pure returns (bytes memory attestationTbs) {
        attestationTbs = new bytes(13 + rawProtectedLength + rawPayloadLength);
        attestationTbs[0] = bytes1(uint8(4 << 5 | 4)); // Outer: 4-length array
        attestationTbs[1] = bytes1(uint8(3 << 5 | 10)); // Context: 10-length string
        attestationTbs[12 + rawProtectedLength] = bytes1(uint8(2 << 5)); // ExternalAAD: 0-length bytes

        string memory sig = "Signature1";
        uint256 dest;
        uint256 sigSrc;
        uint256 protectedSrc;
        uint256 payloadSrc;
        assembly {
            dest := add(attestationTbs, 32)
            sigSrc := add(sig, 32)
            protectedSrc := add(rawProtectedBytes, 32)
            payloadSrc := add(rawPayloadBytes, 32)
        }

        LibBytes.memcpy(dest + 2, sigSrc, 10);
        LibBytes.memcpy(dest + 12, protectedSrc, rawProtectedLength);
        LibBytes.memcpy(dest + 13 + rawProtectedLength, payloadSrc, rawPayloadLength);
    }

    function _parseAttestation(bytes memory attestationTbs) internal pure returns (Ptrs memory) {
        require(attestationTbs.keccak(0, 18) == ATTESTATION_TBS_PREFIX, "invalid attestation prefix");

        CborElement payload = attestationTbs.byteStringAt(18);
        CborElement current = attestationTbs.mapAt(payload.start());

        Ptrs memory ptrs;
        uint256 end = payload.end();
        while (current.end() < end) {
            current = attestationTbs.nextTextString(current);
            bytes32 keyHash = attestationTbs.keccak(current);
            if (keyHash == MODULE_ID_KEY) {
                current = attestationTbs.nextTextString(current);
                ptrs.moduleID = current;
            } else if (keyHash == DIGEST_KEY) {
                current = attestationTbs.nextTextString(current);
                ptrs.digest = current;
            } else if (keyHash == CERTIFICATE_KEY) {
                current = attestationTbs.nextByteString(current);
                ptrs.cert = current;
            } else if (keyHash == PUBLIC_KEY_KEY) {
                current = attestationTbs.nextByteStringOrNull(current);
                ptrs.publicKey = current;
            } else if (keyHash == USER_DATA_KEY) {
                current = attestationTbs.nextByteStringOrNull(current);
                ptrs.userData = current;
            } else if (keyHash == NONCE_KEY) {
                current = attestationTbs.nextByteStringOrNull(current);
                ptrs.nonce = current;
            } else if (keyHash == TIMESTAMP_KEY) {
                current = attestationTbs.nextPositiveInt(current);
                ptrs.timestamp = uint64(current.value());
            } else if (keyHash == CABUNDLE_KEY) {
                current = attestationTbs.nextArray(current);
                ptrs.cabundle = new CborElement[](current.value());
                for (uint256 i = 0; i < ptrs.cabundle.length; i++) {
                    current = attestationTbs.nextByteString(current);
                    ptrs.cabundle[i] = current;
                }
            } else if (keyHash == PCRS_KEY) {
                current = attestationTbs.nextMap(current);
                ptrs.pcrs = new CborElement[](current.value());
                for (uint256 i = 0; i < ptrs.pcrs.length; i++) {
                    current = attestationTbs.nextPositiveInt(current);
                    uint256 key = current.value();
                    require(key < ptrs.pcrs.length, "invalid pcr key value");
                    require(CborElement.unwrap(ptrs.pcrs[key]) == 0, "duplicate pcr key");
                    current = attestationTbs.nextByteString(current);
                    ptrs.pcrs[key] = current;
                }
            } else {
                revert("invalid attestation key");
            }
        }

        return ptrs;
    }

    function _verifySignature(bytes memory pubKey, bytes memory hash, bytes memory sig) internal view {
        require(ECDSA384.verify(ECDSA384Curve.p384(), hash, sig, pubKey), "invalid sig");
    }
}
