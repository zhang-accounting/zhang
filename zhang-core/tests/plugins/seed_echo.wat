;; A processor plugin that reads its `zhang.seed` config entry and appends one comment directive
;; holding it to its input stream, so a test can see the seed the host gave it.
;;
;; Written by hand against the extism kernel ABI like `echo.wat`. `config_get` takes a kernel block
;; holding the key and returns a block holding the value, or 0 when the key is missing (this plugin
;; then traps).
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"seed-echo\"")       ;; 11 bytes
  (data (i32.const 32) "\"0.1.0\"")          ;; 7 bytes
  (data (i32.const 48) "[\"Processor\"]")    ;; 13 bytes
  (data (i32.const 64) "zhang.seed")         ;; 10 bytes
  ;; the comment around the escaped value, appended to the stream: a `Spanned<Directive::Comment>`
  ;; after a comma, then the `]` closing the stream
  (data (i32.const 128) "{\"data\":{\"Comment\":{\"content\":\"")                                ;; 31 bytes
  (data (i32.const 192) "\"}},\"span\":{\"start\":0,\"end\":0,\"content\":\"\",\"filename\":null}}]") ;; 61 bytes

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

  ;; copy `len` bytes of kernel memory at `src` to kernel memory at `dest`; returns the end
  (func $copy_kernel (param $dest i64) (param $src i64) (param $len i64) (result i64)
    (local $i i64)
    (block $done
      (loop $copy
        (br_if $done (i64.ge_u (local.get $i) (local.get $len)))
        (call $store_u8 (i64.add (local.get $dest) (local.get $i)) (call $load_u8 (i64.add (local.get $src) (local.get $i))))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $copy)))
    (i64.add (local.get $dest) (local.get $len)))

  ;; copy `len` bytes at `ptr` into a new kernel block and return it
  (func $block (param $ptr i32) (param $len i32) (result i64)
    (local $block i64)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (drop (call $copy (local.get $block) (local.get $ptr) (local.get $len)))
    (local.get $block))

  (func $output_bytes (param $ptr i32) (param $len i32)
    (call $output_set (call $block (local.get $ptr) (local.get $len)) (i64.extend_i32_u (local.get $len))))

  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 11))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 48) (i32.const 13))
    (i32.const 0))

  ;; return the input stream with one comment appended, holding the kernel block `value` as a JSON string
  (func $append_comment (param $value i64)
    (local $in i64)
    (local $in_len i64)
    (local $value_len i64)
    (local $out i64)
    (local $at i64)
    (local $i i64)
    (local $byte i32)
    (local.set $in (call $input_offset))
    (local.set $in_len (call $input_length))
    (local.set $value_len (call $length (local.get $value)))
    ;; room for the input, a comma, the comment and the value with every byte escaped
    (local.set $out
      (call $alloc (i64.add (i64.add (local.get $in_len) (i64.const 93)) (i64.mul (local.get $value_len) (i64.const 2)))))
    ;; the input is a compact JSON array: copy it without its closing `]`, then a comma unless it is `[]`
    (local.set $at (call $copy_kernel (local.get $out) (local.get $in) (i64.sub (local.get $in_len) (i64.const 1))))
    (if (i64.gt_u (local.get $in_len) (i64.const 2))
      (then
        (call $store_u8 (local.get $at) (i32.const 44))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))))
    (local.set $at (call $copy (local.get $at) (i32.const 128) (i32.const 31)))
    ;; the value holds no raw control characters: escaping `"` and `\` makes it a valid JSON string
    (block $done
      (loop $escape
        (br_if $done (i64.ge_u (local.get $i) (local.get $value_len)))
        (local.set $byte (call $load_u8 (i64.add (local.get $value) (local.get $i))))
        (if (i32.or (i32.eq (local.get $byte) (i32.const 34)) (i32.eq (local.get $byte) (i32.const 92)))
          (then
            (call $store_u8 (local.get $at) (i32.const 92))
            (local.set $at (i64.add (local.get $at) (i64.const 1)))))
        (call $store_u8 (local.get $at) (local.get $byte))
        (local.set $at (i64.add (local.get $at) (i64.const 1)))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $escape)))
    (local.set $at (call $copy (local.get $at) (i32.const 192) (i32.const 61)))
    (call $output_set (local.get $out) (i64.sub (local.get $at) (local.get $out))))

  (func (export "processor") (result i32)
    (local $value i64)
    (local.set $value (call $config_get (call $block (i32.const 64) (i32.const 10))))
    (if (i64.eqz (local.get $value)) (then unreachable))
    (call $append_comment (local.get $value))
    (i32.const 0)))
