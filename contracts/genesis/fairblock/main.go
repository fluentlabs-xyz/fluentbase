package main

import "C"
import (
	"bytes"
	enc "github.com/FairBlock/DistributedIBE/encryption"
	//"bytes"
	//"fmt"
	bls "github.com/drand/kyber-bls12381"
	"unsafe"
)

//go:wasm-module fluentbase_v1preview
//export _read
func _read(*C.char, C.uint, C.uint)

//go:wasm-module fluentbase_v1preview
//export _write
func _write(*C.char, C.uint)

//go:wasm-module fluentbase_v1preview
//export _input_size
func _input_size() C.uint

//go:wasm-module fluentbase_v1preview
//export _exit
func _exit(C.int)

//export deploy
func deploy() {
}

//export main
func main() {
	headerLen := 374
	pkLen := 48
	skLen := 96
	inputSize := _input_size()

	input := make([]C.char, inputSize)
	ptr := (*C.char)(&input[0])

	_read(ptr, C.uint(380), inputSize)

	rawPk := C.GoBytes(unsafe.Pointer(&input[headerLen]), C.int(pkLen))
	pk := bls.NullKyberG1()
	err := pk.UnmarshalBinary(rawPk)

	if err != nil {
		print_err(err)
		return
	}

	rawSk := C.GoBytes(unsafe.Pointer(&input[headerLen+pkLen]), C.int(skLen))

	sk := bls.NullKyberG2()

	err = sk.UnmarshalBinary(rawSk)

	if err != nil {
		print_err(err)
		return
	}

	chiperData := C.GoBytes(unsafe.Pointer(&input[headerLen+pkLen+skLen]), C.int(inputSize)-C.int(pkLen)-C.int(skLen)-C.int(headerLen))

	var plainData bytes.Buffer

	err = enc.Decrypt(pk, sk, &plainData, bytes.NewReader(chiperData))
	if err != nil {
		print_err(err)
		return
	}

	resultLen := C.uint(len(plainData.Bytes()))
	resultPtr := (*C.char)(unsafe.Pointer(&plainData.Bytes()[0]))

	_write(resultPtr, resultLen)
	_exit(0)
}

func print_err(err error) {
	errBytes := []byte(err.Error())

	resultLen := C.uint(len(errBytes))
	resultPtr := (*C.char)(unsafe.Pointer(&errBytes[0]))

	_write(resultPtr, resultLen)
	_exit(-1)
}