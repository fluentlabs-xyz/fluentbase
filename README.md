Fluentbase
==========

## Abstract

WebAssembly (WASM) is an interpreted language and binary format for Web2 (usually) developers.
Our approach describes how to let Web2 developers to be transparently added into Web3 world, but it's a bit challenging.
We like WASM comparing to RISC-V or other binary formats because its well-known and mass-adopted standard that developers like and support.
Also, WASM has self-described binary format (including memory structure, type mapping and the rest) comparing to RISC-V/AMD/Intel binary formats that require some binary-wrappers like EXE or ELF.
But it doesn't mean that WASM is optimal, it still has some tricky non ZK friendly structures that we'd like to avoid to prove.
This is why we need rWASM.

rWASM (Reduced WebAssembly) is a special-modified binary IR (intermediary representation) of WASMi execution.
Literally rWASM is 99% compatible with WASM original bytecode and instruction set, but with a modified binary structure w/o affecting opcode behaviour.
The biggest WASM problem is relative offsets for type mappings, function mappings and block/loop statements (everything that relates to PC offsets).
rWASM binary format has more flatten structure w/o relative offsets and rWASM doesn't require type mapping validator and must be executed as is.
Such flatten structure makes easier to proof correctness of each opcode execution and put several verification steps on developer's hands.

Here is a list of differences:
1. Deterministic function order based their position in the codebase
2. Function indices are replaced with PC offset
3. Block/Loop statements are not supported anymore, instead of this we're using Br/BrIf instructions
4. Break instructions are redesigned to support PC offsets instead of depth-level
5. Sections are removed to simplify binary verification
6. Memory section is computed using WebAssembly instructions instead of sections
7. Global variables are recovered from codebase (no need for section)
8. Type mappings are not required anymore since code is validated
9. Drop/keep is replaced with Get/Set/Tee local instructions

The new binary representation produces 100% valida WASMi's runtime module from binary.
There are several features that are not supported anymore, like exports since the only way to interact with rWASM is only though start section.

List of non-supported features:
1. Export section doesn't work anymore (it can be fixed by injecting router inside)
2. Passive mode data sections (it can be simulated via memory copy)
3. 

## WebAssembly's problems and ways to solve them

Most complicated issues for WASM proofs relate to PC offset calculation.
Here we're defining ways how to avoid such situations by applying binary modifications that help to keep WASM compatibility but let it have more efficient binary structure.
Long story short we need to create flatten binary representation of WASM by keeping backward compatibility with instruction set.

One thing we want to highlight is that WASM is designed to be validated before execution, it means that translation step goes right after validation and translation can't go through if original WASM binary is not valid that helps us to define next statements and assumptions.
1. if WASM binary is valid then rWASM binary is valid too
2. rWASM can't store not possible instruction inside it's binary representation
3. 

### Type section

Creating proof for type mappings is quite expensive because you need to create a lookup table to store information about each parsed binary type

### Global variables
### Function indices
### Memory section
