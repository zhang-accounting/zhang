;; A module growing a function table by 128 Mi elements in a call (`grow_table`): 1 GiB of the host's memory, at 8
;; bytes an element. A table's elements count toward the plugin's memory limit, so under a small one the growth fails
;; the call; a growth by 1000 elements (`grow_table_a_little`) fits.
(module
  (memory 1)
  (table $t 1 funcref)
  ;; 0 when the table grew by `count` elements
  (func $grow (param $count i32) (result i32)
    (i32.eq (table.grow $t (ref.null func) (local.get $count)) (i32.const -1)))
  (func (export "grow_table") (result i32) (call $grow (i32.const 134217728)))
  (func (export "grow_table_a_little") (result i32) (call $grow (i32.const 1000)))
  (func (export "name") (result i32) (i32.const 0)))
