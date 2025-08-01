# Fluentbase Codec Derive

Procedural macros for deriving the `Codec` trait from the
[`fluentbase-codec`](../codec) crate. These macros generate efficient encoding
and decoding implementations that integrate with both `CompactABI` and
`SolidityABI` modes.

```rust
use fluentbase_codec::CompactABI;
use fluentbase_codec_derive::Codec;

#[derive(Codec)]
struct Point {
    x: u32,
    y: u32,
}

let mut buf = bytes::BytesMut::new();
CompactABI::encode(&Point { x: 1, y: 2 }, &mut buf, 0).unwrap();
```

See [`src/lib.rs`](src/lib.rs) for the supported attributes and
customisation options.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
