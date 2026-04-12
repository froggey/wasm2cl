(defpackage :wasm2cl-wasip1
  (:use :cl :wasm2cl)
  (:export #:run

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

(defun run (package-designator env &rest args)
  (let* ((package (or (find-package package-designator)
                      (error "Unknown package ~S" package-designator)))
         (context-create (find-symbol "WASM2CL-CREATE-CONTEXT" package))
         (entry (find-symbol "_start" package))
         (personality (make-instance 'wasip1-personality
                                     :args args
                                     :env env)))
    (catch 'exit
      (funcall entry (funcall context-create personality)))))

(defun |args_sizes_get| (context argc-out-ptr buf-size-out-ptr)
  (i32store context argc-out-ptr 0)
  (i32store context buf-size-out-ptr 0)
  0)

(defun |args_get| (context argv-ptr arg-buf-ptr)
  0)

(defun |environ_sizes_get| (context envc-out-ptr buf-size-out-ptr)
  (i32store context envc-out-ptr 0)
  (i32store context buf-size-out-ptr 0)
  0)

(defun |environ_get| (context envp-ptr arg-buf-ptr)
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
