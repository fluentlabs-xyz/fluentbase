(module
  (func $_sys_read (import "env" "_sys_read") (param i32) (param i32) (param i32))
  (func $_sys_halt (import "env" "_sys_halt") (param i32))
  (func $_sys_write (import "env" "_sys_write") (param i32) (param i32))
  (func $_sizes_get (import "wasi_snapshot_preview1" "environ_sizes_get") (param i32) (param i32) (result i32))

  (table $t 1 funcref)
  (elem declare func $_sys_read $_sys_halt $_sizes_get)

  (func $call-f (param $x i32) (param $y i32) (param $z i32)
    (table.set $t (i32.const 0) (ref.func $_sys_read))
    (call_indirect $t (param i32) (param i32) (param i32) (i32.const 0) (local.get $x) (local.get $y) (local.get $z))
  )

  (func $call-with-return (param $x i32) (param $y i32) (result i32)
    (table.set $t (i32.const 0) (ref.func $_sizes_get))
    (call_indirect $t (param i32) (param i32) (result i32) (i32.const 0) (local.get $x) (local.get $y))
  )

  (func $main
    (i32.const 0)
    (i32.const 0)
    (i32.const 0)
    (call $call-f)
    (i32.const 0)
    (i32.const 0)
    (call $call-with-return)
    (drop)
  )


  (export "main" (func $main))
)

