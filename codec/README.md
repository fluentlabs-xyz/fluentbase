Fluentbase Codec
================

Is our encoding/decoding format that is optimized for random reads.
It is very similar to the standard Solidity ABI encoding with one major difference that is uses static structure, and it doesn't align elements with 32 bytes.
We also support recursive encoding, so everything that can be encoded inside Codec can also be embedded as nested structure.
The idea of this codec is that you can access any first level information w/o reading the rest info.
The only thing you need to know is a type of structure.

## Primitives

Primitives are types that are encoded as is w/o any additional meta information.
Encoding of parameters for these types is zero cost.

List of primitive types:
- `u8/i8/u16/i16/u32/i32/u64/i64` - numbers are encoded in LE format
- `[T;N]` - static arrays

## Non-primitives

Non-primitives is everything that need meta information, like dynamic arrays, hash tables etc.
For dynamic arrays we store information about offset and length (32 bits each).
For hash tables we need to store the same for keys/values arrays, and it makes possible to read keys w/o knowing values and vise versa.

We support next non-primitive types:
- `Vec<T>` - vec of encodable elements
- `HashMap<K,V>` - hashbrown hash map with encodable K & V 
- `HashSet<T>` - hashbrown hash set with encodable T