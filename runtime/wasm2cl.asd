(defsystem :wasm2cl
  :depends-on (#:nibbles #:babel #:sdl2)
  :serial t
  :components ((:file "runtime")
               (:file "wasip1")
               (:file "sdl-graphics")))
