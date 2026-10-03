;; A processor plugin that forges its kernel block header so `length` reports about 2 GiB, then
;; passes that block to `zhang_emit_error`. On a host that trusts the reported length this slices far
;; past the mapped memory and reading it kills the process with SIGBUS. The fix rejects the block as
;; an invalid payload and the load continues.
;;
;; Written by hand against the extism kernel ABI like `emit_error.wat`.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_emit_error" (func $zhang_emit_error (param i64)))

  (memory 1)
  (data (i32.const 0) "\"emit-error-forge\"")  ;; 18 bytes
  (data (i32.const 32) "\"0.1.0\"")            ;; 7 bytes
  (data (i32.const 64) "[\"Processor\"]")      ;; 13 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 18))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (local $p i64)
    ;; a real 8-byte block, then overwrite the most significant byte of its `used` header field (at
    ;; `$p - 1` on wasm32) so the kernel's `length` reports about 2 GiB (0x7f000008)
    (local.set $p (call $alloc (i64.const 8)))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 1)) (i32.const 0x7f))
    (call $zhang_emit_error (local.get $p))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0)))
