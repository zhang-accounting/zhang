;; A router plugin that asks `extism:host/env::http_request` for `http://127.0.0.1:9/` and answers `sent` when the
;; request went out. zhang links its own `http_request` over extism's: it refuses the loopback address whatever
;; `allowed_hosts` grants, which fails the call, while extism's would have tried to connect. See echo.wat for the kernel
;; ABI.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/env" "http_request" (func $http_request (param i64 i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"http-request\"")  ;; name: 14 bytes
  (data (i32.const 32) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 64) "[\"Router\"]")  ;; supported_type: 10 bytes
  (data (i32.const 96) "{\"url\":\"http://127.0.0.1:9/\",\"method\":\"GET\"}")  ;; request: 44 bytes
  (data (i32.const 160) "{\"body\":\"sent\"}")  ;; response: 15 bytes

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

  ;; copy `len` bytes at `ptr` into a new kernel block
  (func $block (param $ptr i32) (param $len i32) (result i64)
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
    (local.get $block))

  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 14))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 10))
    (i32.const 0))

  (func (export "router") (result i32)
    (drop (call $http_request (call $block (i32.const 96) (i32.const 44)) (i64.const 0)))
    (call $output_bytes (i32.const 160) (i32.const 15))
    (i32.const 0)))
