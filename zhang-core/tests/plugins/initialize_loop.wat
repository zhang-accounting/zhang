;; A reactor whose `_initialize` never ends: the module is linked when the host builds an instance, which runs it, before
;; any call. The host must stop it at the plugin's timeout, as for a start function (start_loop.wat).
(module
  (memory 1)
  (func (export "_initialize") (loop $l (br $l)))
  (func (export "name") (result i32) (i32.const 0)))
