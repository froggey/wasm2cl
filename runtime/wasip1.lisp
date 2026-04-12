(defpackage :wasm2cl-wasip1
  (:use :cl :wasm2cl)
  (:export #:run #:run-1

           #:|args_sizes_get| #:|args_get|
           #:|environ_sizes_get| #:|environ_get|
           #:|fd_fdstat_get|
           #:|fd_prestat_get|
           #:|fd_prestat_dir_name|
           #:|fd_filestat_get|
           #:|fd_write| #:|fd_read|
           #:|fd_close|
           #:|path_open|
           #:|proc_exit|))

(in-package :wasm2cl-wasip1)

(defclass wasip1-personality ()
  ((%args :initarg :args)
   (%env :initarg :env)))

(defun run-1 (package-designator &key arguments environment)
  (let* ((package (or (find-package package-designator)
                      (error "Unknown package ~S" package-designator)))
         (context-create (find-symbol "WASM2CL-CREATE-CONTEXT" package))
         (entry (find-symbol "_start" package))
         (personality (make-instance 'wasip1-personality
                                     :args arguments
                                     :env environment)))
    (catch 'exit
      (funcall entry (funcall context-create personality)))))

(defun run (package-designator &rest args)
  (run-1 package-designator
         :arguments (list* (if (packagep package-designator)
                               (package-name package-designator)
                               (string package-designator))
                           args)))

(defun |args_sizes_get| (context argc-out-ptr buf-size-out-ptr)
  (with-slots (%args) (wasm-context-personality context)
    (let ((argc (length %args))
          (buf-size (loop for arg in %args
                          summing (1+ (babel:string-size-in-octets arg)))))
      (i32store context argc-out-ptr argc)
      (i32store context buf-size-out-ptr buf-size)))
  0)

(defun |args_get| (context argv-ptr arg-buf-ptr)
  (with-slots (%args) (wasm-context-personality context)
    (loop with memory = (wasm-context-memory context)
          for arg in %args
          for encoded = (babel:string-to-octets arg)
          do (i32store context argv-ptr arg-buf-ptr)
             (replace memory encoded :start1 arg-buf-ptr)
             (i32store8 context (+ arg-buf-ptr (length encoded)) 0)
             (incf argv-ptr 4)
             (incf arg-buf-ptr (1+ (length encoded)))))
  0)

(defun |environ_sizes_get| (context envc-out-ptr buf-size-out-ptr)
  (with-slots (%env) (wasm-context-personality context)
    (let ((envc (length %env))
          (buf-size (loop for env in %env
                          summing (1+ (babel:string-size-in-octets env)))))
      (i32store context envc-out-ptr envc)
      (i32store context buf-size-out-ptr buf-size)))
  0)

(defun |environ_get| (context envp-ptr buf-ptr)
  (with-slots (%env) (wasm-context-personality context)
    (loop with memory = (wasm-context-memory context)
          for env in %env
          for encoded = (babel:string-to-octets env)
          do (i32store context envp-ptr buf-ptr)
             (replace memory encoded :start1 buf-ptr)
             (i32store8 context (+ buf-ptr (length encoded)) 0)
             (incf envp-ptr 4)
             (incf buf-ptr (1+ (length encoded)))))
  0)

(defun |fd_fdstat_get| (context fd statbuf)
  0)

(defun |fd_write| (context fd iovs n-iovs size-ptr)
  (when (member fd '(1 2)) ; stdout/stderr
    (loop with bytes-written = 0
          for i below n-iovs
          do (let* ((buf (i32load context (+ iovs (* i 8))))
                    (count (i32load context (+ iovs (* i 8) 4)))
                    (data (subseq (wasm2cl::wasm-context-memory context)
                                  buf (+ buf count)))
                    (str (map 'string #'code-char data)))
               (write-string str)
               (incf bytes-written count))
          finally
             (i32store context size-ptr bytes-written)))
  0)

(defun |proc_exit| (context code)
  (declare (ignore context))
  (throw 'exit code))
