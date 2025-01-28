lib.wat: lib.wasm
	wasm2wat lib.wasm > lib.wat || true

lib.wasm: go.mod go.sum main.go
	tinygo build -o lib.wasm --target wasm-unknown github.com/fluentlabs-xyz/fluentbase/examples/fairblock || echo "WARNING: missing TinyGo, build might be outdated"