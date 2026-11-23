;; A module whose start function never ends. It runs while the host builds an instance, before any call: the host must
;; stop it at the plugin's timeout and fail the load saying so, rather than hang every load (and every write's reload).
(module
  (memory 1)
  (func $spin (loop $l (br $l)))
  (start $spin)
  (func (export "name") (result i32) (i32.const 0)))
