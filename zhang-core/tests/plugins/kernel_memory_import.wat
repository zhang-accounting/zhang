;; A module importing the memory of extism's kernel, which would give it raw access to the kernel's state (its blocks,
;; their lengths, the input and output): extism refuses a module importing anything of the kernel but its functions.
(module
  (import "extism:host/env" "memory" (memory 1))
  (func (export "name") (result i32) (i32.const 0)))
