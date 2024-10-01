RUSTFLAGS='-C link-arg=-zstack-size=262144 -C target-feature=+bulk-memory'

.PHONY: sp1
sp1:
	cargo prove build --output-directory ./examples/sp1/elf
