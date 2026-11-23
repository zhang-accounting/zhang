;; A module whose `__wasm_call_ctors` never ends. extism runs it, as a WASI module's constructors, when the first call
;; instantiates the module, before the export it calls: the host must stop it at the call's timeout.
(module
  (memory 1)
  (func (export "__wasm_call_ctors") (loop $l (br $l)))
  (func (export "name") (result i32) (i32.const 0)))
