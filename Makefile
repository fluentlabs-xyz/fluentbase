all: build

.PHONY: build
build:
	# build examples & contracts by triggering "build.rs"
	cargo check --lib #--exclude fluentbase-genesis --workspace --lib
	# build genesis files
	#cd crates/genesis && $(MAKE) # build genesis

.PHONY: examples
examples:
	cd examples && $(MAKE)

.PHONY: clean
clean:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE) clean; fi
	cargo clean

.PHONY: install-nextest
install-nextest:
	@echo "Checking for nextest..."
	@if ! command -v cargo-nextest >/dev/null 2>&1; then \
		echo "Installing nextest..."; \
		case "$$(uname -s)" in \
			Linux*) \
				curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C $${CARGO_HOME:-~/.cargo}/bin ;; \
			Darwin*) \
				curl -LsSf https://get.nexte.st/latest/mac | tar zxf - -C $${CARGO_HOME:-~/.cargo}/bin ;; \
			MINGW*|MSYS*|CYGWIN*) \
				if command -v curl >/dev/null 2>&1 && command -v tar >/dev/null 2>&1; then \
					curl -LsSf https://get.nexte.st/latest/windows-tar | tar zxf - -C $${CARGO_HOME:-~/.cargo}/bin; \
				else \
					powershell -Command " \
						\$$tmp = New-TemporaryFile | Rename-Item -NewName { \$$_ -replace 'tmp$$', 'zip' } -PassThru; \
						Invoke-WebRequest -OutFile \$$tmp https://get.nexte.st/latest/windows; \
						\$$outputDir = if (\$$Env:CARGO_HOME) { Join-Path \$$Env:CARGO_HOME 'bin' } else { '~/.cargo/bin' }; \
						\$$tmp | Expand-Archive -DestinationPath \$$outputDir -Force; \
						\$$tmp | Remove-Item"; \
				fi;; \
			*) \
				echo "Unsupported platform, please install nextest manually: https://nexte.st/docs/installation/pre-built-binaries/"; \
				exit 1;; \
		esac; \
	else \
		echo "nextest already installed"; \
	fi

.PHONY: test
test: install-nextest
	cargo nextest run --no-fail-fast
	@echo "Running doctests (not supported by nextest yet https://github.com/nextest-rs/nextest/issues/16)..."
	cargo test --doc
