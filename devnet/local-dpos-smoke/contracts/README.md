# Vendored Solidity artefacts

These forge JSON files are copies of the build output from the
sibling `solidity-contracts/` repo. To regenerate after a Solidity
change:

    make regen-contracts                                # default sibling path
    SOLIDITY_CONTRACTS_DIR=/path/to/repo make regen-contracts

The files contain `bytecode.object` + `deployedBytecode.object` +
`abi` per forge artefact schema. Storage layout fields are not used
by `genesis-bootstrap` (it uses on-chain ABI calls, not raw slots).

Last regenerated against: solidity-contracts @ `$(cat .vendor-sha)`
(`make regen-contracts` writes the current `git -C $SOLIDITY_CONTRACTS_DIR
rev-parse HEAD` here automatically; no manual backfill).
