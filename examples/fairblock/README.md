# Research Project: Compiling Functions from the FairBlock DistributedIBE to WebAssembly (WASM)

This document outlines the steps to compile functions from the [FairBlock DistributedIBE GitHub Repository](https://github.com/Fairblock/DistributedIBE) using **TinyGo** to WebAssembly (WASM). The goal is to convert Fairblock's cryptography functions into WASM modules. It's needed for integration fairblock with fluent network.

## Prerequisites

To compile the Golang functions into WebAssembly, you will need to install **TinyGo**, which supports compiling Go programs to WASM. TinyGo is a smaller version of Go designed for embedded systems and WebAssembly targets.

### Installing TinyGo
Follow the installation instructions provided in the [official TinyGo documentation](https://tinygo.org/getting-started/). TinyGo supports multiple platforms, so ensure that you select the correct installation steps for your operating system.

## Compilation Steps

### Compiling to WASM

To compile a Golang function from the FairBlock DistributedIBE repository into a `.wasm` file, you need to use the `wasm-unknown` target in TinyGo. Here is the example command that can be used for this purpose:

```bash
tinygo build -o wasm1.wasm -target=wasm-unknown github.com/FairBlock/DistributedIBE
```

This command will generate a `wasm1.wasm` file of approximately **1 MB** in size. The file size may vary slightly depending on the content of the functions you are compiling and any additional dependencies.

### Check result. Converting WASM to WAT

After generating the `.wasm` file, you can convert it to **WAT** (WebAssembly Text Format) using the `wasm2wat` tool, which is part of the WebAssembly Binary Toolkit (WABT). The conversion is done with the following command:

```bash
wasm2wat wasm1.wasm -o wasm1.wat
```

The WAT format is a human-readable representation of the WebAssembly binary format and is useful for inspecting the WebAssembly code.

## Adding Exported Functions in Golang

To compile specific Golang functions to WASM using TinyGo, you need to mark the functions with a special comment. TinyGo recognizes this comment and ensures that the function will be compiled into the WASM output. The comment `//export <function_name>` should be placed above the function definition.

### Example Golang Function with Export Comment

Below is an example of a Golang function from the FairBlock DistributedIBE repository that includes the necessary export comment for TinyGo compilation:

```go
//export decrypt
func decrypt() {
	message := "this is a long message with more than 32 bytes! this is a long message with more than 32 bytes!long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with long message with more than 32 bytes! this is a long message with "
	//message = message
	var plainData bytes.Buffer
	plainData.WriteString(message)

	res, err := DistributedIBE(4, 1, "300", plainData, message)

	if res == false {
		fmt.Println(err)
	}
}
```

This `decrypt()` function will be included in the WASM output due to the `//export decrypt` comment. You can modify the function's behavior as necessary before compiling it to WebAssembly.
