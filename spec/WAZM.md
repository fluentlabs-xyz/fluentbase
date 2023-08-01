WAZM
====

```
[.globals_preamble]
[.data_preamble]
[.main_function] {
    // ...
    call 73 // "_evm_return"
    // ...
    .return
}
[.other_functions] {
    .locals_init {
        i32.const 0
        local_set 0
    }   
    .return
}
[.exit]
```

// state transition function

```
br/br_if <dest_pc>
call <dest_pc>
```


fn_0(locals) -> ret
fn_1(locals) -> ret
fn_2(locals) -> ret

