;; A module whose start function grows its memory by 2000 pages (125 MiB) and writes into every new page, while the
;; host builds an instance, before any call. That memory counts toward the plugin's limit: under a smaller one, building
;; the instance fails, saying so (the limiter was attached once the start function had run, which grew past it).
(module
  (memory 1)
  (func $grow
    (local $page i32)
    (drop (memory.grow (i32.const 2000)))
    (block $done
      (loop $touch
        (br_if $done (i32.gt_u (local.get $page) (i32.const 2000)))
        (i32.store8 (i32.mul (local.get $page) (i32.const 65536)) (i32.const 1))
        (local.set $page (i32.add (local.get $page) (i32.const 1)))
        (br $touch))))
  (start $grow)
  (func (export "name") (result i32) (i32.const 0)))
