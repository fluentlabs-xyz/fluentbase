In this crate we store rewritten WASMI's e2e spec tests from WebAssembly to test rWASM codegen and compilation.
Right now it passes 99% of cases except 4 very tricky corner cases with global variables exports that we can't fully support.
Bringing support of these features makes not much sense for sandbox environment that Fluent runs. 