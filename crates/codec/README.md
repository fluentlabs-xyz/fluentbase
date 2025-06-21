# Fluentbase Codec

A lightweight, no-std compatible codec library optimized for random reads. While similar to Solidity ABI encoding, it
introduces several optimizations and additional features for efficient data access and nested structures.

## Key Features

- No-std compatible
- Configurable byte order and alignment
- Support for Solidity ABI compatibility mode
- Random access to first-level information without full decoding
- Support for recursive encoding of nested structures
- Derive macro support for custom types

## Encoding Modes

The library supports two main encoding modes:

### CompactABI Mode

```rust
CompactABI::encode(&value, &mut buf, 0)
```

- Little-endian byte order
- 4-byte alignment
- Dynamic structure encoding:

  ```
  Header:
    - length (u32): number of elements
    - offset (u32): position in the buffer
    - size (u32): total number of encoded bytes
  Body:
    - encoded elements
  ```

### SolidityABI Mode

```rust
SolidityABI::encode(&value, &mut buf, 0)
```

- Big-endian byte order
- 32-byte alignment (Ethereum compatible)
- Dynamic structure encoding:

  ```
  Header:
    - offset (u32): position in the structure
  Body:
    - length (u32): number of elements
    - encoded elements
  ```

## Type System

### Primitive Types

Primitives are encoded without additional metadata, providing zero-cost encoding:

- Integer types: `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `u64`, `i64`
- Boolean type: `bool`
- Unit type: `()`
- Static arrays: `[T; N]`
- Tuples: `(T,)`, `(T1, T2)`, etc.
- Option type: `Option<T>`
- Large integers: `U128`, `U256`, `I128`, `I256` (from alloy_primitives)

### Non-Primitive Types

These types require additional metadata for encoding:

- `Vec<T>`: Dynamic array of encodable elements
- `HashMap<K,V>`: Hash map with encodable keys and values
- `HashSet<T>`: Hash set with encodable elements

For dynamic types, the codec stores metadata that enables partial reading. For example:

- Vectors store offset and length information
- HashMaps store separate metadata for keys and values, allowing independent access

## Usage Examples

### Basic Structure

```rust
use fluentbase_codec::{Codec, CompactABI};
use bytes::BytesMut;

#[derive(Codec)]
struct Point {
    x: u32,
    y: u32,
}

// Encoding
let point = Point { x: 10, y: 20 };
let mut buf = BytesMut::new();
CompactABI::encode(&point, &mut buf, 0).unwrap();

// Decoding
let decoded: Point = CompactABI::decode(&buf, 0).unwrap();
```

### Dynamic Array Example

```rust
// Vector encoding with metadata
let numbers = vec![1, 2, 3];

// CompactABI encoding (with full metadata)
let mut fluent_buf = BytesMut::new();
CompactABI::encode(&numbers, &mut fluent_buf, 0).unwrap();
// Format: [length:3][offset:12][size:12][1][2][3]

// SolidityABI encoding
let mut solidity_buf = BytesMut::new();
SolidityABI::encode(&numbers, &mut solidity_buf, 0).unwrap();
// Format: [offset:32][length:3][1][2][3]
```

## Important Notes

### Determinism

The encoded binary is not deterministic and should only be used for parameter passing. The encoding order of
non-primitive fields affects the data layout after the header, though decoding will produce the same result regardless
of encoding order.

### Order Sensitivity

The order of encoding operations is significant, especially for non-primitive types, as it affects the final binary
layout.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
fluentbase-codec = "0.1.0"
```

## License

MIT or Apache-2.0

## DEV section

- [x] primitives
