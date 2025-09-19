# NITRO

Nitro integration wrapper. Provides a thin entrypoint that bridges Fluentbase contracts to a Nitro-style execution
environment via host syscalls.

- Entrypoint: main_entry. Parses input, invokes host/native execution where appropriate, and returns results.
- Inputs/Outputs: binary format defined by this crate to match Nitro host expectations.
- Gas/fuel: Accounted by host; entrypoint performs final settlement if needed.

Note: This is environment-specific. Ensure your host exposes the expected Nitro syscalls/APIs.
