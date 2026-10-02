;; A processor plugin that returns its input stream unchanged.
;;
;; Written by hand against the extism kernel ABI, so the tests need no wasm toolchain: extism
;; compiles WAT text directly. Each export returns 0 for success and sets its output to a block of
;; kernel memory. `processor` reuses the input block; the metadata exports copy a constant JSON
;; value from this module's data segment into a new block.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 1)
  (data (i32.const 0) "\"passthrough\"")    ;; 13 bytes
  (data (i32.const 16) "\"0.1.0\"")         ;; 7 bytes
  (data (i32.const 32) "[\"Processor\"]")   ;; 13 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 13))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0)))
