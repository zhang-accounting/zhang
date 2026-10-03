;; A router plugin that mounts the strongest form of the attack against `zhang_query`: it tries to
;; forge the kernel's own `MemoryRoot::length` (so the host's ground-truth bound would become huge)
;; AND forges a block header whose length is within the 1 MiB cap but still runs past the real end of
;; linear memory. If the root length were writable, the host would then slice past the mapped memory
;; and crash. The extism kernel's store exports refuse writes below address 64, so the root stays
;; intact and the query is answered with `invalid_input`. The response body is the raw `zhang_query`
;; result so a test can read it.
;;
;; Written by hand against the extism kernel ABI like `router_query.wat`.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "store_u64" (func $store_u64 (param i64 i64)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_query" (func $zhang_query (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"router-query-root-forge\"")  ;; 25 bytes
  (data (i32.const 32) "\"0.1.0\"")                   ;; 7 bytes
  (data (i32.const 64) "[\"Router\"]")                ;; 10 bytes
  (data (i32.const 96) "{\"body\":\"")                 ;; 9 bytes
  (data (i32.const 112) "\"}")                         ;; 2 bytes

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

  (func (export "name") (result i32)
    (call $output_data (i32.const 0) (i32.const 25))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_data (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_data (i32.const 64) (i32.const 10))
    (i32.const 0))

  ;; answer `{"body": "<escaped zhang_query result>"}`
  (func $respond (param $result i64) (result i32)
    (local $out i64)
    (local $end i64)
    (local.set $out (call $alloc (i64.add (i64.const 11) (i64.mul (call $length (local.get $result)) (i64.const 2)))))
    (local.set $end (call $copy (local.get $out) (i32.const 96) (i32.const 9)))
    (local.set $end (call $escape (local.get $end) (local.get $result) (call $length (local.get $result))))
    (local.set $end (call $copy (local.get $end) (i32.const 112) (i32.const 2)))
    (call $output_set (local.get $out) (i64.sub (local.get $end) (local.get $out)))
    (i32.const 0))

  (func (export "router") (result i32)
    (local $p i64)
    ;; try to forge MemoryRoot::length (at linear address 17) to about 2 GiB -- a plausible value, so
    ;; the host's 4 GiB sanity guard would not catch it. The kernel rejects both writes (addresses
    ;; below 64 are not writable), so the length keeps its real, small value.
    (call $store_u64 (i64.const 17) (i64.const 0x7f000000))
    (call $store_u8 (i64.const 20) (i32.const 0x7f))
    ;; a real 8-byte block, then forge its `used` header field to exactly 1 MiB (within the cap). With
    ;; the block near the start of memory, offset + 1 MiB runs past the real 16-page linear memory, so
    ;; this would overread if the forged root length were trusted.
    (local.set $p (call $alloc (i64.const 8)))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 4)) (i32.const 0x00))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 2)) (i32.const 0x10))
    (call $respond (call $zhang_query (local.get $p)))))
