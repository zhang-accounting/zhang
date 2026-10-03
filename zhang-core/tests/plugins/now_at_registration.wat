;; A processor plugin that calls the `zhang_now` host function only while it registers, from
;; `name`, `version` and `supported_type`, and whose processor returns its input stream unchanged
;; without reading the time.
;;
;; Written by hand against the extism kernel ABI like `echo.wat`. The blocks `zhang_now` returns are
;; dropped.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_now" (func $zhang_now (result i64)))

  (memory 1)
  (data (i32.const 0) "\"now-at-registration\"")  ;; 21 bytes
  (data (i32.const 32) "\"0.1.0\"")               ;; 7 bytes
  (data (i32.const 48) "[\"Processor\"]")         ;; 13 bytes

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
    (drop (call $zhang_now))
    (call $output_bytes (i32.const 0) (i32.const 21))
    (i32.const 0))
  (func (export "version") (result i32)
    (drop (call $zhang_now))
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (drop (call $zhang_now))
    (call $output_bytes (i32.const 48) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0)))
