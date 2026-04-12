(defsystem :wasm2cl
  :depends-on (#:nibbles #:babel)
  :serial t
  :components ((:file "runtime")
               (:file "wasip1")))
