;; A processor plugin that sets its output to a real block with a length of u64::MAX. The kernel's `output_set` checks
;; that the last byte, `offset + length - 1`, is in bounds, and that sum wraps around to `offset - 2`, which is. An
;; unpatched extism then reads the output by that length and the process dies. Patched (vendor/extism), the call fails
;; with an error saying the block lies outside the plugin's memory. See echo.wat for the kernel ABI.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))


  (memory 1)
  (data (i32.const 0) "\"output-forge\"")  ;; name: 14 bytes
  (data (i32.const 32) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 64) "[\"Processor\"]")  ;; supported_type: 13 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 14))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (call $output_set (call $alloc (i64.const 8)) (i64.const -1))
    (i32.const 0)))
