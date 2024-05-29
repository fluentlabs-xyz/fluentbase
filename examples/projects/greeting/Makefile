.DEFAULT_GOAL := all

PKG_NAME := $(shell basename $(PWD))

# Compilation flags
RUSTFLAGS := '-C link-arg=-zstack-size=131072 -C target-feature=+bulk-memory -C opt-level=z -C strip=symbols'

# Paths to the target WASM file and output directory
WASM_TARGET := ./target/wasm32-unknown-unknown/release/$(PKG_NAME).wasm
WASM_OUTPUT_DIR := bin
WASM_OUTPUT_FILE := $(WASM_OUTPUT_DIR)/$(PKG_NAME).wasm
RWASM_OUTPUT_FILE := $(WASM_OUTPUT_DIR)/$(PKG_NAME).rwasm
WASM_WAT_FILE := $(WASM_OUTPUT_DIR)/$(PKG_NAME).wat

# Commands
CARGO_BUILD := cargo build --release --target=wasm32-unknown-unknown --no-default-features
RM := rm -rf
MKDIR := mkdir -p
CP := cp
WASM2WAT := wasm2wat
RWASMC := rwasmc

# Targets
all: build

build: prepare_output_dir
	@echo "Building the project..."
	RUSTFLAGS=$(RUSTFLAGS) $(CARGO_BUILD)

	@echo "Copying the wasm file to the output directory..."
	$(CP) $(WASM_TARGET) $(WASM_OUTPUT_FILE)

	@echo "Converting wasm to wat..."
	$(WASM2WAT) $(WASM_OUTPUT_FILE) > $(WASM_WAT_FILE)

	@echo "Converting wasm to rwasm..."
	$(MAKE) rwasmc


rwasmc: ensure_rwasmc_installed
	@echo "Converting wasm to rwasm..."
	touch "$(RWASM_OUTPUT_FILE)"
	$(RWASMC) --file-in-path="$(WASM_OUTPUT_FILE)" --rwasm-file-out-path="$(RWASM_OUTPUT_FILE)"


prepare_output_dir:
	@echo "Preparing the output directory..."
	$(RM) $(WASM_OUTPUT_DIR)
	$(MKDIR) $(WASM_OUTPUT_DIR)

clean:
	@echo "Cleaning the project..."
	$(RM) $(WASM_OUTPUT_DIR)
	$(RM) ./target

ensure_rwasmc_installed:
	@command -v $(RWASMC) >/dev/null 2>&1 || { \
		echo >&2 "rwasmc is not installed. Installing..."; \
		cargo install --git https://github.com/fluentlabs-xyz/fluentbase fluentbase-bin; \
	}

.PHONY: all build rwasmc prepare_output_dir clean ensure_rwasmc_installed
