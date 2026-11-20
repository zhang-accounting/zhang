;; A processor and router plugin that grows its own memory before it answers: the processor by 512 WebAssembly pages
;; (32 MiB), then passes the stream through; the router by 1024 pages (64 MiB), then answers `grown`. Under a memory limit below that, the growth
;; fails the call with an out-of-memory trap; the plugin never sees `memory.grow` return -1 (it traps itself then, so
;; a host that let the call go on would fail it differently). See echo.wat for the kernel ABI.
(module
  (import "extism:host/env" "input_offset" (func $input_offset (result i64)))
  (import "extism:host/env" "input_length" (func $input_length (result i64)))
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))

  (memory 1)
  (data (i32.const 0) "\"memory-hog\"")  ;; name: 12 bytes
  (data (i32.const 32) "\"0.1.0\"")  ;; version: 7 bytes
  (data (i32.const 64) "[\"Processor\",\"Router\"]")  ;; supported_type: 22 bytes
  (data (i32.const 112) "{\"body\":\"grown\"}")  ;; response: 16 bytes

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
    (call $output_bytes (i32.const 0) (i32.const 12))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 64) (i32.const 22))
    (i32.const 0))

  ;; grow the memory by `pages` pages and write to its last byte
  (func $grow (param $pages i32)
    (if (i32.eq (memory.grow (local.get $pages)) (i32.const -1))
      (then (unreachable)))
    (i32.store8 (i32.sub (i32.mul (memory.size) (i32.const 65536)) (i32.const 1)) (i32.const 1)))

  (func (export "processor") (result i32)
    (call $grow (i32.const 512))
    (call $output_set (call $input_offset) (call $input_length))
    (i32.const 0))

  (func (export "router") (result i32)
    (call $grow (i32.const 1024))
    (call $output_bytes (i32.const 112) (i32.const 16))
    (i32.const 0)))
