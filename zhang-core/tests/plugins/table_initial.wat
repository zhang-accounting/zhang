;; A module declaring a function table of 128 Mi elements, which wasmtime allocates when it builds the instance: 1 GiB
;; of the host's memory, at 8 bytes an element. It counts toward the plugin's memory limit, so under a small one building
;; the instance fails.
(module
  (memory 1)
  (table $t 134217728 funcref)
  (func (export "name") (result i32) (i32.const 0)))
