all: all_sequentially

.PHONY: build_wasm
build_wasm:
	cd ../benchmarks && $(MAKE)

PROFILE=debug# release | debug

.PHONY: build
build:
	#cargo build --profile dev
	@if [ "$(PROFILE)" = "debug" ]; then\
 		cargo build --profile dev; \
 	else \
 	  cargo build --profile release; \
 	fi

.PHONY: universal_token_perf
universal_token_perf:
	perf.sh ../target/$(PROFILE)/universal_token_perf
	mv flame.svg universal_token_perf_flame.svg

.PHONY: all_sequentially
all_sequentially:
	$(MAKE) build
	$(MAKE) universal_token_perf

.PHONY: clean
clean:
	#cargo clean
	rm -rf out.*
	rm -rf perf.data*
