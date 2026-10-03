;; A router plugin that reads the ledger through the router host functions. Every request is
;; answered with the default status and a JSON body `[query, info]`: what `zhang_query` returns for
;; a fixed BQL query, and what `zhang_ledger_info` returns. See router.wat for the escaping.
;;
;; Before answering it reports a problem with `zhang_emit_error`, which a request only logs: the
;; request must still be answered.
;;
;; It is a processor too. Outside a router call both host functions must answer an `Err`: the
;; processor passes the stream through when they do and traps when they do not, so a test can tell
;; that importing them neither breaks the load nor leaks the ledger to a processor.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_query" (func $zhang_query (param i64) (result i64)))
  (import "extism:host/user" "zhang_ledger_info" (func $zhang_ledger_info (result i64)))
  (import "extism:host/user" "zhang_emit_error" (func $zhang_emit_error (param i64)))

  (memory 1)
  (data (i32.const 0) "\"router-query\"")             ;; 14 bytes
  (data (i32.const 16) "\"0.1.0\"")                   ;; 7 bytes
  (data (i32.const 32) "[\"Router\",\"Processor\"]")  ;; 22 bytes
  ;; 74 bytes
  (data (i32.const 64) "SELECT account, sum(position) AS balance GROUP BY account ORDER BY account")
  ;; 56 bytes
  (data (i32.const 144) "{\"headers\":{\"content-type\":\"application/json\"},\"body\":\"[")
  (data (i32.const 208) ",")                          ;; 1 byte
  (data (i32.const 212) "]\"}")                       ;; 3 bytes
  (data (i32.const 224) "{\"message\":\"routed\"}")    ;; 20 bytes

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

  ;; run the fixed query; returns the offset of the result block
  (func $query (result i64)
    (local $bql i64)
    (local.set $bql (call $alloc (i64.const 74)))
    (drop (call $copy (local.get $bql) (i32.const 64) (i32.const 74)))
    (call $zhang_query (local.get $bql)))

  ;; whether a host function result is `{"Err": ...}`: its third byte is `E`
  (func $is_err (param $result i64) (result i32)
    (i32.eq (call $load_u8 (i64.add (local.get $result) (i64.const 2))) (i32.const 69)))

  (func (export "name") (result i32)
    (call $output_data (i32.const 0) (i32.const 14))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_data (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_data (i32.const 32) (i32.const 22))
    (i32.const 0))

  (func (export "processor") (result i32)
    (if (i32.eqz (i32.and (call $is_err (call $query)) (call $is_err (call $zhang_ledger_info))))
      (then unreachable))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0))

  (func (export "router") (result i32)
    (local $query i64)
    (local $info i64)
    (local $out i64)
    (local $end i64)
    (local.set $out (call $alloc (i64.const 20)))
    (drop (call $copy (local.get $out) (i32.const 224) (i32.const 20)))
    (call $zhang_emit_error (local.get $out))
    (local.set $query (call $query))
    (local.set $info (call $zhang_ledger_info))
    ;; room for the fixed parts and both results escaped (at most twice as long)
    (local.set $out (call $alloc
      (i64.add (i64.const 64) (i64.mul (i64.add (call $length (local.get $query)) (call $length (local.get $info))) (i64.const 2)))))
    (local.set $end (call $copy (local.get $out) (i32.const 144) (i32.const 56)))
    (local.set $end (call $escape (local.get $end) (local.get $query) (call $length (local.get $query))))
    (local.set $end (call $copy (local.get $end) (i32.const 208) (i32.const 1)))
    (local.set $end (call $escape (local.get $end) (local.get $info) (call $length (local.get $info))))
    (local.set $end (call $copy (local.get $end) (i32.const 212) (i32.const 3)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0)))
