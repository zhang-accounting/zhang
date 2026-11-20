;; A router plugin that drives `zhang_write_preview` and `zhang_write_commit` with the request's own
;; body, so a test (or curl) sends any batch over HTTP and reads the raw host function result back
;; as the response body. The second character of the request path selects the case:
;;
;; - `p` (`/preview`): `zhang_write_preview(body)`
;; - `c` (`/commit`): `zhang_write_commit(body)`
;; - `g`: a block one byte over the 1 MiB cap, which the host must not read
;; - `f`: a block whose header is forged to report about 2 GiB (see router_query_forge.wat)
;; - `n`: offset 0, which is no block
;; - anything else: an input that is not JSON
;;
;; The body is read out of the request JSON after `"body":"`, with the JSON escapes `\"`, `\\`,
;; `\n`, `\t` and `\/` undone (a test uses no other escape). Its `capabilities` export declares the
;; `writes` config of its directive, a JSON array such as `["transaction","open"]`, so one fixture
;; covers every declaration; without the config it declares nothing. It is a processor too: outside
;; a router call both write functions must answer an `Err`, so the processor passes the stream
;; through when they do and traps when they do not. See router_query.wat for the helpers.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))
  (import "extism:host/user" "zhang_write_preview" (func $zhang_write_preview (param i64) (result i64)))
  (import "extism:host/user" "zhang_write_commit" (func $zhang_write_commit (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"router-write\"")                 ;; 14 bytes
  (data (i32.const 16) "\"0.1.0\"")                       ;; 7 bytes
  (data (i32.const 32) "[\"Router\",\"Processor\"]")      ;; 22 bytes
  ;; 55 bytes
  (data (i32.const 64) "{\"headers\":{\"content-type\":\"application/json\"},\"body\":\"")
  (data (i32.const 128) "\"}")                            ;; 2 bytes
  (data (i32.const 144) "writes")                         ;; 6 bytes
  (data (i32.const 160) "{\"writes\":")                   ;; 10 bytes
  (data (i32.const 176) "}")                              ;; 1 byte
  (data (i32.const 192) "{\"writes\":[]}")                ;; 13 bytes
  (data (i32.const 208) "\"body\":\"")                     ;; 8 bytes
  (data (i32.const 224) "not json")                       ;; 8 bytes
  ;; the batch the processor previews: 43 bytes
  (data (i32.const 256) "{\"directives\":[{\"kind\":\"open\",\"date\":\"x\"}]}")

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

  ;; copy `len` bytes of kernel memory at `src` to `at` as they are; returns where they end
  (func $copy_kernel (param $at i64) (param $src i64) (param $len i64) (result i64)
    (local $i i64)
    (block $done
      (loop $next
        (br_if $done (i64.ge_u (local.get $i) (local.get $len)))
        (call $store_u8 (i64.add (local.get $at) (local.get $i)) (call $load_u8 (i64.add (local.get $src) (local.get $i))))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $next)))
    (i64.add (local.get $at) (local.get $len)))

  ;; a new kernel block holding `len` bytes at `ptr` in this module's memory
  (func $block (param $ptr i32) (param $len i32) (result i64)
    (local $block i64)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (drop (call $copy (local.get $block) (local.get $ptr) (local.get $len)))
    (local.get $block))

  ;; make `len` bytes at `ptr` in this module's memory the output
  (func $output_data (param $ptr i32) (param $len i32)
    (call $output_set (call $block (local.get $ptr) (local.get $len)) (i64.extend_i32_u (local.get $len))))

  ;; whether a host function result is `{"Err": ...}`: its third byte is `E`
  (func $is_err (param $result i64) (result i32)
    (i32.eq (call $load_u8 (i64.add (local.get $result) (i64.const 2))) (i32.const 69)))

  ;; answer `{"headers": {...}, "body": "<escaped result>"}`
  (func $respond (param $result i64) (result i32)
    (local $out i64)
    (local $end i64)
    (local.set $out (call $alloc (i64.add (i64.const 57) (i64.mul (call $length (local.get $result)) (i64.const 2)))))
    (local.set $end (call $copy (local.get $out) (i32.const 64) (i32.const 55)))
    (local.set $end (call $escape (local.get $end) (local.get $result) (call $length (local.get $result))))
    (local.set $end (call $copy (local.get $end) (i32.const 128) (i32.const 2)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0))

  ;; the second character of the request path: the `"` closing the method is found from byte 11 of
  ;; `{"method":"...","path":"/x"`, and the path's second character is 11 bytes after it
  (func $case (result i32)
    (local $at i64)
    (local.set $at (i64.add (call $input_offset) (i64.const 11)))
    (block $done
      (loop $next
        (br_if $done (i32.eq (call $load_u8 (local.get $at)) (i32.const 34)))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))
        (br $next)))
    (call $load_u8 (i64.add (local.get $at) (i64.const 11))))

  ;; where the request body's text starts in the input: right after the first `"body":"`; 0 when
  ;; there is none
  (func $body_start (result i64)
    (local $at i64)
    (local $end i64)
    (local $i i64)
    (local.set $at (call $input_offset))
    (local.set $end (i64.add (local.get $at) (call $input_length)))
    (block $none
      (loop $next
        (br_if $none (i64.gt_u (i64.add (local.get $at) (i64.const 8)) (local.get $end)))
        (local.set $i (i64.const 0))
        (block $mismatch
          (loop $compare
            (br_if $mismatch
              (i32.ne
                (call $load_u8 (i64.add (local.get $at) (local.get $i)))
                (i32.load8_u (i32.add (i32.const 208) (i32.wrap_i64 (local.get $i))))))
            (local.set $i (i64.add (local.get $i) (i64.const 1)))
            (br_if $compare (i64.lt_u (local.get $i) (i64.const 8)))
            (return (i64.add (local.get $at) (i64.const 8)))))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))
        (br $next)))
    (i64.const 0))

  ;; the JSON escape `\x` undone: the byte it stands for, for `"`, `\`, `/`, `n` and `t`; any other
  ;; byte as it is
  (func $unescaped (param $byte i32) (result i32)
    (if (i32.eq (local.get $byte) (i32.const 110)) (then (return (i32.const 10))))
    (if (i32.eq (local.get $byte) (i32.const 116)) (then (return (i32.const 9))))
    (local.get $byte))

  ;; the body's text from `src` with its escapes undone, stored at `dst` unless `dst` is 0; returns
  ;; how many bytes it holds
  (func $decode (param $src i64) (param $dst i64) (result i64)
    (local $n i64)
    (local $byte i32)
    (block $done
      (loop $next
        (local.set $byte (call $load_u8 (local.get $src)))
        (br_if $done (i32.eq (local.get $byte) (i32.const 34)))
        (if (i32.eq (local.get $byte) (i32.const 92))
          (then
            (local.set $src (i64.add (local.get $src) (i64.const 1)))
            (local.set $byte (call $unescaped (call $load_u8 (local.get $src))))))
        (if (i64.ne (local.get $dst) (i64.const 0))
          (then (call $store_u8 (i64.add (local.get $dst) (local.get $n)) (local.get $byte))))
        (local.set $n (i64.add (local.get $n) (i64.const 1)))
        (local.set $src (i64.add (local.get $src) (i64.const 1)))
        (br $next)))
    (local.get $n))

  ;; a new kernel block holding the request body, its JSON escapes undone
  (func $body (result i64)
    (local $src i64)
    (local $out i64)
    (local.set $src (call $body_start))
    (if (i64.eqz (local.get $src)) (then (return (call $block (i32.const 224) (i32.const 8)))))
    (local.set $out (call $alloc (call $decode (local.get $src) (i64.const 0))))
    (drop (call $decode (local.get $src) (local.get $out)))
    (local.get $out))

  (func (export "name") (result i32)
    (call $output_data (i32.const 0) (i32.const 14))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_data (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_data (i32.const 32) (i32.const 22))
    (i32.const 0))

  ;; `{"writes": <the `writes` config>}`, or no writes without the config
  (func (export "capabilities") (result i32)
    (local $value i64)
    (local $out i64)
    (local $end i64)
    (local.set $value (call $config_get (call $block (i32.const 144) (i32.const 6))))
    (if (i64.eqz (local.get $value))
      (then
        (call $output_data (i32.const 192) (i32.const 13))
        (return (i32.const 0))))
    (local.set $out (call $alloc (i64.add (i64.const 11) (call $length (local.get $value)))))
    (local.set $end (call $copy (local.get $out) (i32.const 160) (i32.const 10)))
    ;; the value is the JSON array as written in the directive: copied as it is
    (local.set $end (call $copy_kernel (local.get $end) (local.get $value) (call $length (local.get $value))))
    (local.set $end (call $copy (local.get $end) (i32.const 176) (i32.const 1)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0))

  (func (export "processor") (result i32)
    (if (i32.eqz (i32.and
          (call $is_err (call $zhang_write_preview (call $block (i32.const 256) (i32.const 43))))
          (call $is_err (call $zhang_write_commit (call $block (i32.const 256) (i32.const 43))))))
      (then unreachable))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0))

  (func (export "router") (result i32)
    (local $case i32)
    (local $p i64)
    (local.set $case (call $case))
    (if (i32.eq (local.get $case) (i32.const 112))  ;; 'p'
      (then (return (call $respond (call $zhang_write_preview (call $body))))))
    (if (i32.eq (local.get $case) (i32.const 99))  ;; 'c'
      (then (return (call $respond (call $zhang_write_commit (call $body))))))
    (if (i32.eq (local.get $case) (i32.const 103))  ;; 'g': a block one byte over the cap
      (then (return (call $respond (call $zhang_write_preview (call $alloc (i64.const 1048577)))))))
    (if (i32.eq (local.get $case) (i32.const 102))  ;; 'f': a forged block header, about 2 GiB
      (then
        (local.set $p (call $alloc (i64.const 8)))
        (call $store_u8 (i64.sub (local.get $p) (i64.const 1)) (i32.const 0x7f))
        (return (call $respond (call $zhang_write_preview (local.get $p))))))
    (if (i32.eq (local.get $case) (i32.const 110))  ;; 'n': no block at all
      (then (return (call $respond (call $zhang_write_preview (i64.const 0))))))
    (call $respond (call $zhang_write_preview (call $block (i32.const 224) (i32.const 8))))))
