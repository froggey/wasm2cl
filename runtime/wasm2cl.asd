(defsystem :wasm2cl
  :depends-on (#:nibbles #:babel #-mezzano #:sdl2)
  :serial t
  :components ((:file "runtime")
               (:file "wasip1")
               #-mezzano
               (:file "sdl-graphics")
               #+mezzano
               (:file "mezzano-graphics")))
