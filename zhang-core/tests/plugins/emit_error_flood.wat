;; A plugin flooding `zhang_emit_error`. `flood_large` reports the same 1 MiB payload 200 times, the most a payload may
;; be (the reviewer's probe: 200 MiB of the host's memory under a plugin limit of 32 MiB); `flood_small` reports a small
;; error 1500 times. The host keeps at most 1000 errors, 1 MiB in all, and says how many it dropped.
(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/user" "zhang_emit_error" (func $emit (param i64)))
  (memory 1)
  (data (i32.const 0) "{\"message\":\"")
  (data (i32.const 16) "{\"message\":\"payee is missing\"}")  ;; 30 bytes

  ;; a kernel block of `len` bytes holding {"message":"xxx…"}
  (func $large (param $len i64) (result i64)
    (local $b i64) (local $i i64)
    (local.set $b (call $alloc (local.get $len)))
    (block $d1 (loop $l1
      (br_if $d1 (i64.ge_u (local.get $i) (i64.const 12)))
      (call $store_u8 (i64.add (local.get $b) (local.get $i)) (i32.load8_u (i32.wrap_i64 (local.get $i))))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $l1)))
    (block $d2 (loop $l2
      (br_if $d2 (i64.ge_u (local.get $i) (i64.sub (local.get $len) (i64.const 2))))
      (call $store_u8 (i64.add (local.get $b) (local.get $i)) (i32.const 120))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $l2)))
    (call $store_u8 (i64.add (local.get $b) (local.get $i)) (i32.const 34))
    (call $store_u8 (i64.add (local.get $b) (i64.add (local.get $i) (i64.const 1))) (i32.const 125))
    (local.get $b))

  ;; a kernel block holding the small payload
  (func $small (result i64)
    (local $b i64) (local $i i64)
    (local.set $b (call $alloc (i64.const 30)))
    (block $d (loop $l
      (br_if $d (i64.ge_u (local.get $i) (i64.const 30)))
      (call $store_u8 (i64.add (local.get $b) (local.get $i)) (i32.load8_u (i32.add (i32.const 16) (i32.wrap_i64 (local.get $i)))))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $l)))
    (local.get $b))

  ;; report the block `b` `times` times
  (func $emit_times (param $b i64) (param $times i32)
    (local $k i32)
    (block $d (loop $l
      (br_if $d (i32.ge_u (local.get $k) (local.get $times)))
      (call $emit (local.get $b))
      (local.set $k (i32.add (local.get $k) (i32.const 1)))
      (br $l))))

  (func (export "flood_large") (result i32)
    (call $emit_times (call $large (i64.const 1048576)) (i32.const 200))
    (i32.const 0))
  (func (export "flood_small") (result i32)
    (call $emit_times (call $small) (i32.const 1500))
    (i32.const 0))
  (func (export "name") (result i32) (i32.const 0)))
