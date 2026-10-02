;; A processor plugin that reads its `zhang.plugin` config entry and returns it as the content of a
;; single comment directive, dropping the input stream, so a test can see exactly what the host
;; passed. See echo.wat for the kernel ABI. `config_get` takes a kernel block holding the key and
;; returns a block holding the value, or 0 when the key is missing (this plugin then traps).
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "length" (func $length (param i64) (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))

  (memory 1)
  (data (i32.const 0) "\"config-echo\"")     ;; 13 bytes
  (data (i32.const 16) "\"0.1.0\"")          ;; 7 bytes
  (data (i32.const 32) "[\"Processor\"]")    ;; 13 bytes
  (data (i32.const 48) "zhang.plugin")       ;; 12 bytes
  ;; the output around the escaped value: one `Spanned<Directive::Comment>` in an array
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
    (call $output_bytes (i32.const 0) (i32.const 13))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 13))
    (i32.const 0))

  (func (export "processor") (result i32)
    (local $value i64)
    (local $value_len i64)
    (local $out i64)
    (local $at i64)
    (local $i i64)
    (local $byte i32)
    (local.set $value (call $config_get (call $block (i32.const 48) (i32.const 12))))
    (if (i64.eqz (local.get $value)) (then unreachable))
    (local.set $value_len (call $length (local.get $value)))
    ;; room for the prefix, the suffix and the value with every byte escaped
    (local.set $out (call $alloc (i64.add (i64.const 93) (i64.mul (local.get $value_len) (i64.const 2)))))
    (local.set $at (call $copy (local.get $out) (i32.const 64) (i32.const 32)))
    ;; the value is compact JSON, so it holds no raw control characters: escaping `"` and `\`
    ;; makes it a valid JSON string
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
    (local.set $at (call $copy (local.get $at) (i32.const 128) (i32.const 61)))
    (call $output_set (local.get $out) (i64.sub (local.get $at) (local.get $out)))
    (i32.const 0)))
