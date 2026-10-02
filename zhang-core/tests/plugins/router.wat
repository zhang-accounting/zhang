;; A router plugin that answers every request with the request itself: status 201, a custom
;; `x-echo` header, and the request JSON the host sent as a JSON body, so a test sees exactly the
;; method, path, query, headers and body the plugin received. See echo.wat for the kernel ABI.
;;
;; The response body is a JSON string, so the request JSON is escaped into it. The host's JSON
;; holds no raw control characters, so escaping `"` and `\` is enough.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 1)
  (data (i32.const 0) "\"router-echo\"")    ;; 13 bytes
  (data (i32.const 16) "\"0.1.0\"")         ;; 7 bytes
  (data (i32.const 32) "[\"Router\"]")      ;; 10 bytes
  ;; 86 bytes
  (data (i32.const 64) "{\"status\":201,\"headers\":{\"x-echo\":\"router\",\"content-type\":\"application/json\"},\"body\":\"")
  (data (i32.const 160) "\"}")              ;; 2 bytes

  ;; copy `len` bytes at `ptr` in this module's memory to kernel memory at `at`; returns where they end
  (func $copy (param $at i64) (param $ptr i32) (param $len i32) (result i64)
    (local $i i32)
    (block $done
      (loop $next
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (call $store_u8
          (i64.add (local.get $at) (i64.extend_i32_u (local.get $i)))
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (i64.add (local.get $at) (i64.extend_i32_u (local.get $len))))

  ;; copy `len` bytes of kernel memory at `src` to `at`, escaping `"` and `\` for a JSON string;
  ;; returns where they end
  (func $escape (param $at i64) (param $src i64) (param $len i64) (result i64)
    (local $i i64)
    (local $byte i32)
    (block $done
      (loop $next
        (br_if $done (i64.ge_u (local.get $i) (local.get $len)))
        (local.set $byte (call $load_u8 (i64.add (local.get $src) (local.get $i))))
        (if (i32.or (i32.eq (local.get $byte) (i32.const 34)) (i32.eq (local.get $byte) (i32.const 92)))
          (then
            (call $store_u8 (local.get $at) (i32.const 92))
            (local.set $at (i64.add (local.get $at) (i64.const 1)))))
        (call $store_u8 (local.get $at) (local.get $byte))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $next)))
    (local.get $at))

  ;; make `len` bytes at `ptr` in this module's memory the output
  (func $output_data (param $ptr i32) (param $len i32)
    (local $block i64)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (drop (call $copy (local.get $block) (local.get $ptr) (local.get $len)))
    (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))

  (func (export "name") (result i32)
    (call $output_data (i32.const 0) (i32.const 13))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_data (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_data (i32.const 32) (i32.const 10))
    (i32.const 0))

  (func (export "router") (result i32)
    (local $out i64)
    (local $end i64)
    ;; room for the prefix, the escaped request (at most twice as long) and the suffix
    (local.set $out (call $alloc (i64.add (i64.const 88) (i64.mul (call $input_length) (i64.const 2)))))
    (local.set $end (call $copy (local.get $out) (i32.const 64) (i32.const 86)))
    (local.set $end (call $escape (local.get $end) (call $input_offset) (call $input_length)))
    (local.set $end (call $copy (local.get $end) (i32.const 160) (i32.const 2)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0)))
