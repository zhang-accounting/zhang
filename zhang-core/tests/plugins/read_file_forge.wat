;; A processor plugin that forges its kernel block header so `length` reports about 2 GiB, then
;; passes that block to `zhang_read_file` as the path, and returns what the host answered as the
;; content of a single comment directive. On a host that trusts the reported length this slices far
;; past the mapped memory and reading it kills the process with SIGBUS. The fix answers `invalid`
;; and the load continues.
;;
;; Written by hand against the extism kernel ABI; see emit_error_forge.wat for the forge and
;; files.wat for how the answer becomes the output.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/user" "zhang_read_file" (func $zhang_read_file (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"forge\"")           ;; 7 bytes
  (data (i32.const 16) "\"0.1.0\"")          ;; 7 bytes
  (data (i32.const 32) "[\"Processor\"]")    ;; 13 bytes
  ;; the output around the escaped result: one `Spanned<Directive::Comment>` in an array
  (data (i32.const 64) "[{\"data\":{\"Comment\":{\"content\":\"")                               ;; 32 bytes
  (data (i32.const 128) "\"}},\"span\":{\"start\":0,\"end\":0,\"content\":\"\",\"filename\":null}}]") ;; 61 bytes

  ;; copy `len` bytes at `ptr` in this module's memory to kernel memory at `dest`; returns the end
  (func $copy (param $dest i64) (param $ptr i32) (param $len i32) (result i64)
    (local $i i32)
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (call $store_u8
          (i64.add (local.get $dest) (i64.extend_i32_u (local.get $i)))
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $copy)))
    (i64.add (local.get $dest) (i64.extend_i32_u (local.get $len))))

  ;; copy `len` bytes at `ptr` into a new kernel block and return it
  (func $block (param $ptr i32) (param $len i32) (result i64)
    (local $block i64)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (drop (call $copy (local.get $block) (local.get $ptr) (local.get $len)))
    (local.get $block))

  (func $output_bytes (param $ptr i32) (param $len i32)
    (call $output_set (call $block (local.get $ptr) (local.get $len)) (i64.extend_i32_u (local.get $len))))

  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 7))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (local $p i64)
    (local $result i64)
    (local $result_len i64)
    (local $out i64)
    (local $at i64)
    (local $i i64)
    (local $byte i32)
    ;; a real 8-byte block, then overwrite the most significant byte of its `used` header field (at
    ;; `$p - 1` on wasm32) so the kernel's `length` reports about 2 GiB (0x7f000008)
    (local.set $p (call $alloc (i64.const 8)))
    (call $store_u8 (i64.sub (local.get $p) (i64.const 1)) (i32.const 0x7f))
    (local.set $result (call $zhang_read_file (local.get $p)))
    (local.set $result_len (call $length (local.get $result)))
    ;; room for the prefix, the suffix and the result with every byte escaped
    (local.set $out (call $alloc (i64.add (i64.const 93) (i64.mul (local.get $result_len) (i64.const 2)))))
    (local.set $at (call $copy (local.get $out) (i32.const 64) (i32.const 32)))
    ;; the result is compact JSON, so it holds no raw control characters: escaping `"` and `\`
    ;; makes it a valid JSON string
    (block $done
      (loop $escape
        (br_if $done (i64.ge_u (local.get $i) (local.get $result_len)))
        (local.set $byte (call $load_u8 (i64.add (local.get $result) (local.get $i))))
        (if (i32.or (i32.eq (local.get $byte) (i32.const 34)) (i32.eq (local.get $byte) (i32.const 92)))
          (then
            (call $store_u8 (local.get $at) (i32.const 92))
            (local.set $at (i64.add (local.get $at) (i64.const 1)))))
        (call $store_u8 (local.get $at) (local.get $byte))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $escape)))
    (local.set $at (call $copy (local.get $at) (i32.const 128) (i32.const 61)))
    (call $output_set (local.get $out) (i64.sub (local.get $at) (local.get $out)))
    (i32.const 0)))
