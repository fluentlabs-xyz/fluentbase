# BinaryLoader
## FluentBase Loader Crate

The `loader` crate server as loading a binary into memory validating it by hash.
It is easy to program loader by using kecakk256 hash function from crypto crate.
Note after loading, we have to let the zkvm runs in dynamic memory mode.
It is also easy to load the binary and runs it in dynamic memory mod without loader.
The hard part is to run the binary after loader finished.
 In fact it is unclear whether we can do. Note that in rwasm engine,we do not have memory mode. We simply continue to run the binary by replacing rwasm module from loader to the target binary module.
 So, it remains whether this will gives a correct trace for zkvm to generate proof.
 Note that only zkvm works in dynamic mode.

 Right now the memory region are inconsistent between loader and the target binary.
 Because when the binary is in static mode, the memory region is commited at the start of the program.So the memory chip start with elements section commited.
So the tracer fo rwasm engine need to prepare the trace exact same as the binary run in static mode.

It does not need to be pre-commited, however the memory region should be same as the binary run in static mode.
 An easy way to do is after running the loader, we can copy the binary'section  into its corresponding memory region. elements to elements,table to table,etc.
