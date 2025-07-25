# Fluentbase Types

Primitive types and data structures shared across all Fluentbase crates. The
crate exposes the `Address` and `Bytes` types along with the runtime context
objects and helper enums used for error handling and contract execution.

Most types are `no_std` friendly and re-export the `rwasm` core primitives when
the `std` feature is disabled.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
