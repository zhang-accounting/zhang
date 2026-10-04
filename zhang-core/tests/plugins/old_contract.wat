;; A processor and mapper written against the oldest directive contract of ABI v1: it knows no `pad`
;; directive, like a plugin built against a zhang-ast from before `pad`, whose deserializer fails on
;; one. Both exports fail when their input holds a `pad` directive (the JSON `{"Pad":`), and return
;; their input unchanged otherwise: the stream for `processor`, `[directive]` for `mapper`.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "load_u8" (func $load_u8 (param i64) (result i32)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 1)
  (data (i32.const 0) "\"old-contract\"")           ;; 14 bytes
  (data (i32.const 16) "\"0.1.0\"")                 ;; 7 bytes
  (data (i32.const 32) "[\"Processor\",\"Mapper\"]") ;; 22 bytes
  (data (i32.const 64) "{\"Pad\":")                 ;; 7 bytes: what an unknown `pad` directive starts with

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

  ;; whether the input holds the 7 bytes at 64
  (func $holds_a_pad (result i32)
    (local $offset i64)
    (local $length i64)
    (local $i i64)
    (local $j i32)
    (local.set $offset (call $input_offset))
    (local.set $length (call $input_length))
    (block $absent
      (loop $scan
        (br_if $absent (i64.gt_u (i64.add (local.get $i) (i64.const 7)) (local.get $length)))
        (local.set $j (i32.const 0))
        (block $differs
          (loop $compare
            (if (i32.eq (local.get $j) (i32.const 7))
              (then (return (i32.const 1))))
            (br_if $differs
              (i32.ne
                (call $load_u8 (i64.add (i64.add (local.get $offset) (local.get $i)) (i64.extend_i32_u (local.get $j))))
                (i32.load8_u (i32.add (i32.const 64) (local.get $j)))))
            (local.set $j (i32.add (local.get $j) (i32.const 1)))
            (br $compare)))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $scan)))
    (i32.const 0))

  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 14))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 22))
    (i32.const 0))

  (func (export "processor") (result i32)
    (if (call $holds_a_pad) (then (return (i32.const 1))))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0))

  ;; `[` + the directive + `]`
  (func (export "mapper") (result i32)
    (local $offset i64)
    (local $length i64)
    (local $block i64)
    (local $i i64)
    (if (call $holds_a_pad) (then (return (i32.const 1))))
    (local.set $offset (call $input_offset))
    (local.set $length (call $input_length))
    (local.set $block (call $alloc (i64.add (local.get $length) (i64.const 2))))
    (call $store_u8 (local.get $block) (i32.const 91))
    (block $done
      (loop $copy
        (br_if $done (i64.ge_u (local.get $i) (local.get $length)))
        (call $store_u8
          (i64.add (i64.add (local.get $block) (i64.const 1)) (local.get $i))
          (call $load_u8 (i64.add (local.get $offset) (local.get $i))))
        (local.set $i (i64.add (local.get $i) (i64.const 1)))
        (br $copy)))
    (call $store_u8 (i64.add (i64.add (local.get $block) (i64.const 1)) (local.get $length)) (i32.const 93))
    (call $output_set (local.get $block) (i64.add (local.get $length) (i64.const 2)))
    (i32.const 0)))
