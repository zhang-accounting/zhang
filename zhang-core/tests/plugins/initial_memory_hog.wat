;; A processor plugin that starts with 4000 WebAssembly pages of memory (250 MiB) and grows by 4000 more before it
;; passes the stream through: 500 MiB in all. Under the default limit of 256 MiB the growth must fail the call. extism
;; as published counted only the growth (it reset its count after instantiating the module), so this ran at twice the
;; limit. See echo.wat for the kernel ABI.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 4000)
  (data (i32.const 0) "\"initial-memory-hog\"")  ;; name: 20 bytes
  (data (i32.const 48) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 80) "[\"Processor\"]")  ;; supported_type: 13 bytes

  ;; copy `len` bytes at `ptr` into a new kernel block and make it the output
  (func $output_bytes (param $ptr i32) (param $len i32)
    (local $block i64)
    (local $i i32)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (call $store_u8
          (i64.add (local.get $block) (i64.extend_i32_u (local.get $i)))
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $copy)))
    (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))

  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 20))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 48) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 80) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (if (i32.eq (memory.grow (i32.const 4000)) (i32.const -1))
      (then (unreachable)))
    (i32.store8 (i32.sub (i32.mul (memory.size) (i32.const 65536)) (i32.const 1)) (i32.const 1))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0)))
