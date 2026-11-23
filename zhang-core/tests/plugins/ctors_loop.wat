;; A module whose `__wasm_call_ctors` never ends. extism runs it, as a WASI module's constructors, when the host builds
;; an instance, before any call: the host must stop it at the plugin's timeout, as for a start function (start_loop.wat).
(module
  (memory 1)
  (func (export "__wasm_call_ctors") (loop $l (br $l)))
  (func (export "name") (result i32) (i32.const 0)))
