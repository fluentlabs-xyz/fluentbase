# Colors
GREEN := \033[0;32m
YELLOW := \033[0;33m
BLUE := \033[0;34m
NC := \033[0m

# Configuration
WASM_TARGET := wasm32-unknown-unknown
OUTPUT_DIR := wasm-out
MODULE ?= primitives

# Workspace target directory (корневая директория проекта)
WORKSPACE_TARGET := ../../target

# Files
OLD_WASM := $(OUTPUT_DIR)/$(MODULE)_old.wasm
NEW_WASM := $(OUTPUT_DIR)/$(MODULE)_new.wasm
OLD_DISASM := $(OUTPUT_DIR)/$(MODULE)_old.disasm
NEW_DISASM := $(OUTPUT_DIR)/$(MODULE)_new.disasm
OLD_WAT := $(OUTPUT_DIR)/$(MODULE)_old.wat
NEW_WAT := $(OUTPUT_DIR)/$(MODULE)_new.wat

.PHONY: compare
compare: build-both analyze ## Build and compare both versions

.PHONY: compare-existing
compare-existing: ## Compare two existing WASM files (usage: make compare-existing WASM1=file1.wasm WASM2=file2.wasm)
	@if [ -z "$(WASM1)" ] || [ -z "$(WASM2)" ]; then \
		echo "$(YELLOW)Usage: make compare-existing WASM1=path/to/file1.wasm WASM2=path/to/file2.wasm$(NC)"; \
		exit 1; \
	fi
	@if [ ! -f "$(WASM1)" ]; then \
		echo "$(YELLOW)Error: File $(WASM1) not found$(NC)"; \
		exit 1; \
	fi
	@if [ ! -f "$(WASM2)" ]; then \
		echo "$(YELLOW)Error: File $(WASM2) not found$(NC)"; \
		exit 1; \
	fi

	@echo "$(BLUE)=== Comparing existing WASM files ===$(NC)"
	@echo "File 1: $(WASM1)"
	@echo "File 2: $(WASM2)"

	@echo ""
	@echo "$(BLUE)=== Size Comparison ===$(NC)"
	@size1=$$(stat -f%z $(WASM1) 2>/dev/null || stat -c%s $(WASM1)); \
	size2=$$(stat -f%z $(WASM2) 2>/dev/null || stat -c%s $(WASM2)); \
	diff=$$((size1 - size2)); \
	if [ $$size1 -ne 0 ]; then \
		percent=$$((diff * 100 / size1)); \
	else \
		percent=0; \
	fi; \
	echo "File 1: $$size1 bytes"; \
	echo "File 2: $$size2 bytes"; \
	echo "Difference: $$diff bytes ($$percent% reduction)"

	@echo ""
	@echo "$(BLUE)=== Twiggy Diff ===$(NC)"
	@twiggy diff $(WASM1) $(WASM2) | head -40 || echo "Install twiggy: cargo install twiggy"

	@echo ""
	@echo "$(BLUE)=== Generating temporary disassembly ===$(NC)"
	@mkdir -p $(OUTPUT_DIR)/temp
	@wasm-objdump -d $(WASM1) > $(OUTPUT_DIR)/temp/wasm1.disasm
	@wasm-objdump -d $(WASM2) > $(OUTPUT_DIR)/temp/wasm2.disasm

	@echo ""
	@echo "$(BLUE)=== Generating temporary WAT files ===$(NC)"
	@wasm2wat $(WASM1) -o $(OUTPUT_DIR)/temp/wasm1.wat
	@wasm2wat $(WASM2) -o $(OUTPUT_DIR)/temp/wasm2.wat
	@echo "Generated WAT files for comparison"

	@echo ""
	@echo "$(BLUE)=== Total opcodes ===$(NC)"
	@echo "File 1 total: $$(grep -c '^ [0-9a-f]\+:' $(OUTPUT_DIR)/temp/wasm1.disasm)"
	@echo "File 2 total: $$(grep -c '^ [0-9a-f]\+:' $(OUTPUT_DIR)/temp/wasm2.disasm)"

	@echo ""
	@echo "$(BLUE)=== Function count comparison ===$(NC)"
	@func1=$$(grep -c '^[0-9a-f]* func\[' $(OUTPUT_DIR)/temp/wasm1.disasm); \
	func2=$$(grep -c '^[0-9a-f]* func\[' $(OUTPUT_DIR)/temp/wasm2.disasm); \
	echo "File 1 functions: $$func1"; \
	echo "File 2 functions: $$func2"

	@# Cleanup temp files
	@rm -rf $(OUTPUT_DIR)/temp

.PHONY: build-both
build-both: ## Build old and new versions
	@echo "$(YELLOW)Building WASM files...$(NC)"
	@mkdir -p $(OUTPUT_DIR)

	@echo "Building old version..."
	@cargo build --target $(WASM_TARGET) --example $(MODULE)_old --release
	@cp $(WORKSPACE_TARGET)/$(WASM_TARGET)/release/examples/$(MODULE)_old.wasm $(OLD_WASM)

	@echo "Building new version..."
	@cargo build --target $(WASM_TARGET) --example $(MODULE)_new --release
	@cp $(WORKSPACE_TARGET)/$(WASM_TARGET)/release/examples/$(MODULE)_new.wasm $(NEW_WASM)

	@echo "$(GREEN)✓ Built both versions$(NC)"

.PHONY: analyze
analyze: ## Analyze and compare
	@echo ""
	@echo "$(BLUE)=== Size Comparison ===$(NC)"
	@old_size=$$(stat -f%z $(OLD_WASM) 2>/dev/null || stat -c%s $(OLD_WASM)); \
	new_size=$$(stat -f%z $(NEW_WASM) 2>/dev/null || stat -c%s $(NEW_WASM)); \
	diff=$$((old_size - new_size)); \
	percent=$$((diff * 100 / old_size)); \
	echo "Old: $$old_size bytes"; \
	echo "New: $$new_size bytes"; \
	echo "Difference: $$diff bytes ($$percent% reduction)"

	@echo ""
	@echo "$(BLUE)=== Twiggy Diff ===$(NC)"
	@twiggy diff $(OLD_WASM) $(NEW_WASM) | head -20 || echo "Install twiggy: cargo install twiggy"

	@echo ""
	@echo "$(BLUE)=== Generating disassembly ===$(NC)"
	@wasm-objdump -d $(OLD_WASM) > $(OLD_DISASM)
	@wasm-objdump -d $(NEW_WASM) > $(NEW_DISASM)

	@echo ""
	@echo "$(BLUE)=== Generating WAT files ===$(NC)"
	@wasm2wat $(OLD_WASM) -o $(OLD_WAT)
	@wasm2wat $(NEW_WASM) -o $(NEW_WAT)
	@echo "Generated $(OLD_WAT) and $(NEW_WAT)"

	@echo ""
	@echo "$(BLUE)=== Main function analysis ===$(NC)"
	@old_count=$$(sed -n '/<main>:/,/^[0-9a-f]* func\[/p' $(OLD_DISASM) | grep -c '^ [0-9a-f]\+:' || echo 0); \
	new_count=$$(sed -n '/<main>:/,/^[0-9a-f]* func\[/p' $(NEW_DISASM) | grep -c '^ [0-9a-f]\+:' || echo 0); \
	diff=$$((old_count - new_count)); \
	if [ $$old_count -gt 0 ]; then \
		percent=$$((diff * 100 / old_count)); \
		echo "Old main(): $$old_count opcodes"; \
		echo "New main(): $$new_count opcodes"; \
		echo "Improvement: $$diff opcodes ($$percent% reduction)"; \
	else \
		echo "Main function not found"; \
	fi

	@echo ""
	@echo "$(BLUE)=== Total opcodes ===$(NC)"
	@echo "Old total: $$(grep -c '^ [0-9a-f]\+:' $(OLD_DISASM))"
	@echo "New total: $$(grep -c '^ [0-9a-f]\+:' $(NEW_DISASM))"

.PHONY: show-main
show-main: ## Show main function disassembly
	@echo "$(YELLOW)Old main():$(NC)"
	@sed -n '/<main>:/,/^[0-9a-f]* func\[/p' $(OLD_DISASM) | head -50
	@echo ""
	@echo "$(YELLOW)New main():$(NC)"
	@sed -n '/<main>:/,/^[0-9a-f]* func\[/p' $(NEW_DISASM) | head -50

.PHONY: show-main-wat
show-main-wat: ## Show main function in WAT format
	@echo "$(YELLOW)Old main() in WAT:$(NC)"
	@sed -n '/(func $$main/,/^[[:space:]]*)/p' $(OLD_WAT) | head -50
	@echo ""
	@echo "$(YELLOW)New main() in WAT:$(NC)"
	@sed -n '/(func $$main/,/^[[:space:]]*)/p' $(NEW_WAT) | head -50

.PHONY: clean
clean: ## Clean artifacts
	@cargo clean
	@rm -rf $(OUTPUT_DIR)

.PHONY: help
help: ## Show help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-15s\033[0m %s\n", $$1, $$2}'
