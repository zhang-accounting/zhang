;; A router plugin answering every request with the response a hostile plugin would send: an HTML page whose
;; script calls zhang's API with the visitor's session, a `set-cookie` replacing that session, its own
;; `content-security-policy`, CORS headers, `clear-site-data`, a `location` and a `refresh` leading to another origin.
;; zhang must send the page sandboxed, with only the headers it allows. See echo.wat for the kernel ABI.
;;
;; The data segments hold JSON; their lengths are in the comments.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 1)
  (data (i32.const 0) "\"router-hostile\"")  ;; name: 16 bytes
  (data (i32.const 32) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 64) "[\"Router\"]")  ;; supported_type: 10 bytes
  (data (i32.const 96) "{\"status\":200,\"headers\":{\"content-type\":\"text/html; charset=utf-8\",\"set-cookie\":\"zhang_session=evil; Path=/; HttpOnly\",\"content-security-policy\":\"default-src *; report-uri https://evil.example/csp\",\"access-control-allow-origin\":\"https://evil.example\",\"access-control-allow-credentials\":\"true\",\"clear-site-data\":\"\\\"*\\\"\",\"location\":\"https://evil.example/\",\"refresh\":\"0; url=https://evil.example/\",\"x-frame-options\":\"ALLOWALL\",\"cache-control\":\"no-store\"},\"body\":\"<script>fetch('/api/auth/app/code', {method: 'POST', credentials: 'include'})</script>\"}")  ;; response: 548 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 16))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 10))
    (i32.const 0))

  (func (export "router") (result i32)
    (call $output_bytes (i32.const 96) (i32.const 548))
    (i32.const 0)))
