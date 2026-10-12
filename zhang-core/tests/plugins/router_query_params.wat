;; A router plugin that drives `zhang_query_params` and `zhang_query_schema` with a fixed input
;; per request path, so one fixture covers every case. The second character of the request path
;; (byte 25 of the request JSON, `{"method":"GET","path":"/x"`) selects the input below; the
;; response body is the raw result of the host function, so a test can read it:
;;
;; - `a`: a date, a str and a decimal parameter (the decimal as a JSON number)
;; - `b`: a parameter bound to null
;; - `c`: a parameter of an unknown type
;; - `d`: a parameter whose value is not of the shape its type takes
;; - `e`: a parameter the query uses but the input does not bind
;; - `f`: an input that is not JSON
;; - `g`: an input one byte over the 1 MiB cap
;; - anything else: `zhang_query_schema`
;;
;; It is a processor too: outside a router call both host functions must answer an `Err`, so the
;; processor passes the stream through when they do and traps when they do not. See
;; router_query.wat for the helpers.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_query_params" (func $zhang_query_params (param i64) (result i64)))
  (import "extism:host/user" "zhang_query_schema" (func $zhang_query_schema (result i64)))

  (memory 1)
  (data (i32.const 0) "\"router-query-params\"")      ;; 21 bytes
  (data (i32.const 32) "\"0.1.0\"")                   ;; 7 bytes
  (data (i32.const 48) "[\"Router\",\"Processor\"]")  ;; 22 bytes
  (data (i32.const 80) "{\"body\":\"")                 ;; 9 bytes
  (data (i32.const 96) "\"}")                          ;; 2 bytes
  ;; case 'a': 220 bytes
  (data (i32.const 256) "{\"bql\":\"SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min\",\"params\":{\"from\":{\"type\":\"date\",\"value\":\"2024-01-01\"},\"who\":{\"type\":\"str\",\"value\":\"lunch\"},\"min\":{\"type\":\"decimal\",\"value\":5.5}}}")
  ;; case 'b': 89 bytes
  (data (i32.const 512) "{\"bql\":\"SELECT date WHERE narration = :who\",\"params\":{\"who\":{\"type\":\"str\",\"value\":null}}}")
  ;; case 'c': 90 bytes
  (data (i32.const 768) "{\"bql\":\"SELECT date WHERE narration = :who\",\"params\":{\"who\":{\"type\":\"money\",\"value\":\"x\"}}}")
  ;; case 'd': 92 bytes
  (data (i32.const 1024) "{\"bql\":\"SELECT date WHERE date >= :from\",\"params\":{\"from\":{\"type\":\"date\",\"value\":20240101}}}")
  ;; case 'e': 53 bytes
  (data (i32.const 1280) "{\"bql\":\"SELECT date WHERE date >= :from\",\"params\":{}}")
  ;; case 'f': 11 bytes
  (data (i32.const 1536) "SELECT date")

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

  ;; copy `len` bytes of kernel memory at `src` to `at`, escaping `"` and `\`; returns where they end
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

  ;; pass `len` bytes at `ptr` in this module's memory to `zhang_query_params`; returns the result block
  (func $query (param $ptr i32) (param $len i32) (result i64)
    (local $block i64)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (drop (call $copy (local.get $block) (local.get $ptr) (local.get $len)))
    (call $zhang_query_params (local.get $block)))

  ;; whether a host function result is `{"Err": ...}`: its third byte is `E`
  (func $is_err (param $result i64) (result i32)
    (i32.eq (call $load_u8 (i64.add (local.get $result) (i64.const 2))) (i32.const 69)))

  ;; answer `{"body": "<escaped result>"}`
  (func $respond (param $result i64) (result i32)
    (local $out i64)
    (local $end i64)
    (local.set $out (call $alloc (i64.add (i64.const 11) (i64.mul (call $length (local.get $result)) (i64.const 2)))))
    (local.set $end (call $copy (local.get $out) (i32.const 80) (i32.const 9)))
    (local.set $end (call $escape (local.get $end) (local.get $result) (call $length (local.get $result))))
    (local.set $end (call $copy (local.get $end) (i32.const 96) (i32.const 2)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0))

  (func (export "name") (result i32)
    (call $output_data (i32.const 0) (i32.const 21))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_data (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_data (i32.const 48) (i32.const 22))
    (i32.const 0))

  (func (export "processor") (result i32)
    (if (i32.eqz (i32.and (call $is_err (call $query (i32.const 512) (i32.const 89))) (call $is_err (call $zhang_query_schema))))
      (then unreachable))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0))

  (func (export "router") (result i32)
    (local $case i32)
    (local.set $case (call $load_u8 (i64.add (call $input_offset) (i64.const 25))))
    (if (i32.eq (local.get $case) (i32.const 97))  ;; 'a'
      (then (return (call $respond (call $query (i32.const 256) (i32.const 220))))))
    (if (i32.eq (local.get $case) (i32.const 98))  ;; 'b'
      (then (return (call $respond (call $query (i32.const 512) (i32.const 89))))))
    (if (i32.eq (local.get $case) (i32.const 99))  ;; 'c'
      (then (return (call $respond (call $query (i32.const 768) (i32.const 90))))))
    (if (i32.eq (local.get $case) (i32.const 100))  ;; 'd'
      (then (return (call $respond (call $query (i32.const 1024) (i32.const 92))))))
    (if (i32.eq (local.get $case) (i32.const 101))  ;; 'e'
      (then (return (call $respond (call $query (i32.const 1280) (i32.const 53))))))
    (if (i32.eq (local.get $case) (i32.const 102))  ;; 'f'
      (then (return (call $respond (call $query (i32.const 1536) (i32.const 11))))))
    (if (i32.eq (local.get $case) (i32.const 103))  ;; 'g': a block one byte over the cap, which the host must not read
      (then (return (call $respond (call $zhang_query_params (call $alloc (i64.const 1048577)))))))
    (call $respond (call $zhang_query_schema))))
