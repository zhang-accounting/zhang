;; A processor plugin that mounts the strongest form of the attack: it tries to forge the kernel's
;; own `MemoryRoot::length` (so the host's ground-truth bound would become huge) AND forges a block
;; header whose length is within the 1 MiB cap but still runs past the real end of linear memory. If
;; the root length were writable, the host would then slice past the mapped memory and crash. The
;; extism kernel's store exports refuse writes below address 64, so the root stays intact, the block
;; is rejected as an invalid payload, and the load continues.
;;
;; Written by hand against the extism kernel ABI like `emit_error.wat`.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "store_u64" (func $store_u64 (param i64 i64)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_emit_error" (func $zhang_emit_error (param i64)))

  (memory 1)
  (data (i32.const 0) "\"emit-error-root-forge\"")  ;; 23 bytes
  (data (i32.const 32) "\"0.1.0\"")                 ;; 7 bytes
  (data (i32.const 64) "[\"Processor\"]")           ;; 13 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 23))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (local $p i64)
    ;; try to forge MemoryRoot::length (at linear address 17) to about 2 GiB -- a plausible value, so
    ;; the host's 4 GiB sanity guard would not catch it. The kernel rejects both writes (addresses
    ;; below 64 are not writable), so the length keeps its real, small value.
    (call $store_u64 (i64.const 17) (i64.const 0x7f000000))
    (call $store_u8 (i64.const 20) (i32.const 0x7f))
    ;; a real 8-byte block, then forge its `used` header field to exactly 1 MiB (within the cap). With
    ;; the block near the start of memory, offset + 1 MiB runs past the real 16-page linear memory,
    ;; so this would overread if the forged root length were trusted.
    (local.set $p (call $alloc (i64.const 8)))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 4)) (i32.const 0x00))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 2)) (i32.const 0x10))
    (call $zhang_emit_error (local.get $p))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0)))
