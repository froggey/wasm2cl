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

(defconstant +success+ 0 "No error occurred. System call completed successfully.")
(defconstant +err-2big+ 1 "Argument list too long.")
(defconstant +err-acces+ 2 "Permission denied.")
(defconstant +err-addrinuse+ 3 "Address in use.")
(defconstant +err-addrnotavail+ 4 "Address not available.")
(defconstant +err-afnosupport+ 5 "Address family not supported.")
(defconstant +err-again+ 6 "Resource unavailable, or operation would block.")
(defconstant +err-already+ 7 "Connection already in progress.")
(defconstant +err-badf+ 8 "Bad file descriptor.")
(defconstant +err-badmsg+ 9 "Bad message.")
(defconstant +err-busy+ 10 "Device or resource busy.")
(defconstant +err-canceled+ 11 "Operation canceled.")
(defconstant +err-child+ 12 "No child processes.")
(defconstant +err-connaborted+ 13 "Connection aborted.")
(defconstant +err-connrefused+ 14 "Connection refused.")
(defconstant +err-connreset+ 15 "Connection reset.")
(defconstant +err-deadlk+ 16 "Resource deadlock would occur.")
(defconstant +err-destaddrreq+ 17 "Destination address required.")
(defconstant +err-dom+ 18 "Mathematics argument out of domain of function.")
(defconstant +err-dquot+ 19 "Reserved.")
(defconstant +err-exist+ 20 "File exists.")
(defconstant +err-fault+ 21 "Bad address.")
(defconstant +err-fbig+ 22 "File too large.")
(defconstant +err-hostunreach+ 23 "Host is unreachable.")
(defconstant +err-idrm+ 24 "Identifier removed.")
(defconstant +err-ilseq+ 25 "Illegal byte sequence.")
(defconstant +err-inprogress+ 26 "Operation in progress.")
(defconstant +err-intr+ 27 "Interrupted function.")
(defconstant +err-inval+ 28 "Invalid argument.")
(defconstant +err-io+ 29 "I/O error.")
(defconstant +err-isconn+ 30 "Socket is connected.")
(defconstant +err-isdir+ 31 "Is a directory.")
(defconstant +err-loop+ 32 "Too many levels of symbolic links.")
(defconstant +err-mfile+ 33 "File descriptor value too large.")
(defconstant +err-mlink+ 34 "Too many links.")
(defconstant +err-msgsize+ 35 "Message too large.")
(defconstant +err-multihop+ 36 "Reserved.")
(defconstant +err-nametoolong+ 37 "Filename too long.")
(defconstant +err-netdown+ 38 "Network is down.")
(defconstant +err-netreset+ 39 "Connection aborted by network.")
(defconstant +err-netunreach+ 40 "Network unreachable.")
(defconstant +err-nfile+ 41 "Too many files open in system.")
(defconstant +err-nobufs+ 42 "No buffer space available.")
(defconstant +err-nodev+ 43 "No such device.")
(defconstant +err-noent+ 44 "No such file or directory.")
(defconstant +err-noexec+ 45 "Executable file format error.")
(defconstant +err-nolck+ 46 "No locks available.")
(defconstant +err-nolink+ 47 "Reserved.")
(defconstant +err-nomem+ 48 "Not enough space.")
(defconstant +err-nomsg+ 49 "No message of the desired type.")
(defconstant +err-noprotoopt+ 50 "Protocol not available.")
(defconstant +err-nospc+ 51 "No space left on device.")
(defconstant +err-nosys+ 52 "Function not supported.")
(defconstant +err-notconn+ 53 "The socket is not connected.")
(defconstant +err-notdir+ 54 "Not a directory or a symbolic link to a directory.")
(defconstant +err-notempty+ 55 "Directory not empty.")
(defconstant +err-notrecoverable+ 56 "State not recoverable.")
(defconstant +err-notsock+ 57 "Not a socket.")
(defconstant +err-notsup+ 58 "Not supported, or operation not supported on socket.")
(defconstant +err-notty+ 59 "Inappropriate I/O control operation.")
(defconstant +err-nxio+ 60 "No such device or address.")
(defconstant +err-overflow+ 61 "Value too large to be stored in data type.")
(defconstant +err-ownerdead+ 62 "Previous owner died.")
(defconstant +err-perm+ 63 "Operation not permitted.")
(defconstant +err-pipe+ 64 "Broken pipe.")
(defconstant +err-proto+ 65 "Protocol error.")
(defconstant +err-protonosupport+ 66 "Protocol not supported.")
(defconstant +err-prototype+ 67 "Protocol wrong type for socket.")
(defconstant +err-range+ 68 "Result too large.")
(defconstant +err-rofs+ 69 "Read-only file system.")
(defconstant +err-spipe+ 70 "Invalid seek.")
(defconstant +err-srch+ 71 "No such process.")
(defconstant +err-stale+ 72 "Reserved.")
(defconstant +err-timedout+ 73 "Connection timed out.")
(defconstant +err-txtbsy+ 74 "Text file busy.")
(defconstant +err-xdev+ 75 "Cross-device link.")
(defconstant +err-notcapable+ 76 "Extension: Capabilities insufficient.")

(defconstant +filetype-unknown+ 0 "The type of the file descriptor or file is unknown or is different from any of the other types specified.")
(defconstant +filetype-block-device+ 1 "The file descriptor or file refers to a block device inode.")
(defconstant +filetype-character-device+ 2 "The file descriptor or file refers to a character device inode.")
(defconstant +filetype-directory+ 3 "The file descriptor or file refers to a directory inode.")
(defconstant +filetype-regular-file+ 4 "The file descriptor or file refers to a regular file inode.")
(defconstant +filetype-socket-dgram+ 5 "The file descriptor or file refers to a datagram socket.")
(defconstant +filetype-socket-stream+ 6 "The file descriptor or file refers to a byte-stream socket.")
(defconstant +filetype-symbolic-link+ 7 "The file refers to a symbolic link inode.")

(defconstant +oflags-creat+ (ash 1 0) "Create file if it does not exist.")
(defconstant +oflags-directory+ (ash 1 1) "Fail if not a directory.")
(defconstant +oflags-excl+ (ash 1 2) "Fail if file already exists.")
(defconstant +oflags-trunc+ (ash 1 3) "Truncate file to size 0.")

(defconstant +fdflags-append+ (ash 1 0)
  "Append mode: Data written to the file is always appended to the file's end.")
(defconstant +fdflags-dsync+ (ash 1 1)
  "Write according to synchronized I/O data integrity completion. Only the data stored in the file is synchronized.")
(defconstant +fdflags-nonblock+ (ash 1 2)
  "Non-blocking mode.")
(defconstant +fdflags-rsync+ (ash 1 3)
  "Synchronized read I/O operations.")
(defconstant +fdflags-sync+ (ash 1 4)
  "Write according to synchronized I/O file integrity completion. In
addition to synchronizing the data stored in the file, the implementation
may also synchronously update the file's metadata.")

(defconstant +lookupflags-symlink-follow+ (ash 1 0)
  "As long as the resolved path corresponds to a symbolic link, it is expanded.")

(defconstant +rights-fd-datasync+ (ash 1 0)
  "The right to invoke `fd_datasync`.
If `path_open` is set, includes the right to invoke
`path_open` with `fdflags::dsync`.")
(defconstant +rights-fd-read+ (ash 1 1)
  "The right to invoke `fd_read` and `sock_recv`.
If `rights::fd_seek` is set, includes the right to invoke `fd_pread`.")
(defconstant +rights-fd-seek+ (ash 1 2)
  "The right to invoke `fd_seek`. This flag implies `rights::fd_tell`.")
(defconstant +rights-fd-fdstat-set-flags+ (ash 1 3)
  "The right to invoke `fd_fdstat_set_flags`.")
(defconstant +rights-fd-sync+ (ash 1 4)
  "The right to invoke `fd_sync`.
If `path_open` is set, includes the right to invoke
`path_open` with `fdflags::rsync` and `fdflags::dsync`.")
(defconstant +rights-fd-tell+ (ash 1 5)
  "The right to invoke `fd_seek` in such a way that the file offset
remains unaltered (i.e., `whence::cur` with offset zero), or to
invoke `fd_tell`.")
(defconstant +rights-fd-write+ (ash 1 6)
  "The right to invoke `fd_write` and `sock_send`.
If `rights::fd_seek` is set, includes the right to invoke `fd_pwrite`.")
(defconstant +rights-fd-advise+ (ash 1 7)
  "The right to invoke `fd_advise`.")
(defconstant +rights-fd-allocate+ (ash 1 8)
  "The right to invoke `fd_allocate`.")
(defconstant +rights-path-create-directory+ (ash 1 9)
  "The right to invoke `path_create_directory`.")
(defconstant +rights-path-create-file+ (ash 1 10)
  "If `path_open` is set, the right to invoke `path_open` with `oflags::creat`.")
(defconstant +rights-path-link-source+ (ash 1 11)
  "The right to invoke `path_link` with the file descriptor as the
source directory.")
(defconstant +rights-path-link-target+ (ash 1 12)
  "The right to invoke `path_link` with the file descriptor as the
target directory.")
(defconstant +rights-path-open+ (ash 1 13)
  "The right to invoke `path_open`.")
(defconstant +rights-fd-readdir+ (ash 1 14)
  "The right to invoke `fd_readdir`.")
(defconstant +rights-path-readlink+ (ash 1 15)
  "The right to invoke `path_readlink`.")
(defconstant +rights-path-rename-source+ (ash 1 16)
  "The right to invoke `path_rename` with the file descriptor as the source directory.")
(defconstant +rights-path-rename-target+ (ash 1 17)
  "The right to invoke `path_rename` with the file descriptor as the target directory.")
(defconstant +rights-path-filestat-get+ (ash 1 18)
  "The right to invoke `path_filestat_get`.")
(defconstant +rights-path-filestat-set-size+ (ash 1 19)
  "The right to change a file's size (there is no `path_filestat_set_size`).
If `path_open` is set, includes the right to invoke `path_open` with `oflags::trunc`.")
(defconstant +rights-path-filestat-set-times+ (ash 1 20)
  "The right to invoke `path_filestat_set_times`.")
(defconstant +rights-fd-filestat-get+ (ash 1 21)
  "The right to invoke `fd_filestat_get`.")
(defconstant +rights-fd-filestat-set-size+ (ash 1 22)
  "The right to invoke `fd_filestat_set_size`.")
(defconstant +rights-fd-filestat-set-times+ (ash 1 23)
  "The right to invoke `fd_filestat_set_times`.")
(defconstant +rights-path-symlink+ (ash 1 24)
  "The right to invoke `path_symlink`.")
(defconstant +rights-path-remove-directory+ (ash 1 25)
  "The right to invoke `path_remove_directory`.")
(defconstant +rights-path-unlink-file+ (ash 1 26)
  "The right to invoke `path_unlink_file`.")
(defconstant +rights-poll-fd-readwrite+ (ash 1 27)
  "If `rights::fd_read` is set, includes the right to invoke `poll_oneoff` to subscribe to `eventtype::fd_read`.
If `rights::fd_write` is set, includes the right to invoke `poll_oneoff` to subscribe to `eventtype::fd_write`.")
(defconstant +rights-sock-shutdown+ (ash 1 28)
  "The right to invoke `sock_shutdown`.")
(defconstant +rights-sock-accept+ (ash 1 29)
  "The right to invoke `sock_accept`.")

(defclass output-stream-file () ())

(defclass binary-file ()
  ((%path :initarg :path)
   (%stream :initarg :stream)))

(defclass preopened-path ()
  ((%path :initarg :path)))

(defclass wasip1-personality ()
  ((%args :initarg :args)
   (%env :initarg :env)
   (%fd-table :initarg :fd-table)))

(defgeneric close-file (file))

(defmethod close-file ((file output-stream-file))
  nil)

(defmethod close-file ((file preopened-path))
  nil)

(defmethod close-file ((file binary-file))
  (close (slot-value file '%stream)))

(defun resolve-fd (context fd)
  (with-slots (%fd-table) (wasm-context-personality context)
    (if (and (<= 0 fd) (< fd (length %fd-table)))
        (aref %fd-table fd)
        nil)))

(defun drop-personality (personality)
  ;; Close all open files.
  (loop for file across (slot-value personality '%fd-table)
        do (when file
             (close-file file))))

(defun run-1 (package-designator personality)
  (let* ((package (or (find-package package-designator)
                      (error "Unknown package ~S" package-designator)))
         (context-create (find-symbol "WASM2CL-CREATE-CONTEXT" package))
         (entry (find-symbol "_start" package)))
    (unwind-protect
         (catch 'exit
           (funcall entry (funcall context-create personality)))
      (drop-personality personality))))

(defun run (package-designator &rest args)
  (let* ((fd-table (make-array 4
                               :initial-contents (list nil ;; stdin
                                                       ;; stdout
                                                       (make-instance 'output-stream-file)
                                                       ;; stderr
                                                       (make-instance 'output-stream-file)
                                                       (make-instance 'preopened-path :path "/"))
                               :adjustable t
                               :fill-pointer t))
         (personality (make-instance 'wasip1-personality
                                     :args (list* (if (packagep package-designator)
                                                      (package-name package-designator)
                                                      (string package-designator))
                                                  args)
                                     :env '()
                                     :fd-table fd-table)))
    (run-1 package-designator personality)))

(defun |args_sizes_get| (context argc-out-ptr buf-size-out-ptr)
  (with-slots (%args) (wasm-context-personality context)
    (let ((argc (length %args))
          (buf-size (loop for arg in %args
                          summing (1+ (babel:string-size-in-octets arg)))))
      (i32store context argc-out-ptr argc)
      (i32store context buf-size-out-ptr buf-size)))
  +success+)

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
  +success+)

(defun |environ_sizes_get| (context envc-out-ptr buf-size-out-ptr)
  (with-slots (%env) (wasm-context-personality context)
    (let ((envc (length %env))
          (buf-size (loop for env in %env
                          summing (1+ (babel:string-size-in-octets env)))))
      (i32store context envc-out-ptr envc)
      (i32store context buf-size-out-ptr buf-size)))
  +success+)

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
  +success+)

(defun |fd_prestat_get| (context fd statbuf)
  (let ((file (resolve-fd context fd)))
    (cond ((typep file 'preopened-path)
           (i32store8 context statbuf 0) ; tag (directory)
           (i32store context (+ statbuf 4)
                     (babel:string-size-in-octets (slot-value file '%path)))
           +success+)
          (t
           +err-badf+))))

(defun |fd_prestat_dir_name| (context fd buf len)
  (declare (ignore len))
  (let ((file (resolve-fd context fd)))
    (cond ((typep file 'preopened-path)
           (replace (wasm-context-memory context)
                    (babel:string-to-octets (slot-value file '%path))
                    :start1 buf)
           +success+)
          (t
           +err-badf+))))

(defun |fd_fdstat_get| (context fd statbuf)
  (let ((file (resolve-fd context fd)))
    (cond (file
           ;; filetype
           (i32store8 context (+ statbuf 0) (etypecase file
                                              (output-stream-file +filetype-character-device+)
                                              (binary-file +filetype-regular-file+)
                                              (preopened-path +filetype-directory+)))
           ;; flags (fdflags)
           (i32store16 context (+ statbuf 2) 0)
           ;; fs_rights_base
           (i64store context (+ statbuf 8) #xFFFFFFFFFFFFFFFF)
           ;; fs_rights_inheriting
           (i64store context (+ statbuf 8) #xFFFFFFFFFFFFFFFF)
           +success+)
          (t
           +err-badf+))))

(defgeneric stat-file (file))

(defmethod stat-file ((file binary-file))
  (list :filetype +filetype-regular-file+
        :size (file-length (slot-value file '%stream))))

(defun |fd_filestat_get| (context fd statbuf)
  (let ((file (resolve-fd context fd)))
    (if file
        (let ((stat (stat-file file)))
          (i64store context (+ statbuf 0) (getf stat :dev 0))
          (i64store context (+ statbuf 8) (getf stat :ino 0))
          (i32store8 context (+ statbuf 16) (getf stat :filetype +filetype-unknown+))
          (i64store context (+ statbuf 24) (getf stat :nlink 1))
          (i64store context (+ statbuf 32) (getf stat :size 0))
          (i64store context (+ statbuf 40) (getf stat :atim 0))
          (i64store context (+ statbuf 48) (getf stat :mtim 0))
          (i64store context (+ statbuf 56) (getf stat :ctim 0))
          +success+)
        +err-badf+)))

(defgeneric do-write (context file iovs))

(defmethod do-write (context (file output-stream-file) iovs)
  (loop with bytes-written = 0
        for (buf . count) in iovs
        do (let* ((data (subseq (wasm2cl::wasm-context-memory context)
                                buf (+ buf count)))
                  (str (map 'string #'code-char data)))
             (write-string str)
             (incf bytes-written count))
        finally
           (return bytes-written)))

(defun |fd_write| (context fd iovs n-iovs size-ptr)
  (let ((iovs (loop for i below n-iovs
                    for addr = (i32load context (+ iovs (* i 8)))
                    for count = (i32load context (+ iovs (* i 8) 4))
                    collect (cons addr count)))
        (file (resolve-fd context fd)))
    (cond (file
           (let ((count (do-write context file iovs)))
             (i32store context size-ptr count))
           +success+)
          (t
           +err-badf+))))

(defgeneric do-read (context file iovs))

(defmethod do-read (context (file binary-file) iovs)
  (loop with bytes-written = 0
        with stream = (slot-value file '%stream)
        for (buf . count) in iovs
        do
           (let* ((pos (read-sequence (wasm2cl::wasm-context-memory context)
                                      stream
                                      :start buf :end (+ buf count)))
                  (elts (- pos buf)))
             (incf bytes-written elts)
             (unless (eql elts count)
               (loop-finish)))
        finally
           (return bytes-written)))

(defun |fd_read| (context fd iovs n-iovs size-ptr)
  (let ((iovs (loop for i below n-iovs
                    for addr = (i32load context (+ iovs (* i 8)))
                    for count = (i32load context (+ iovs (* i 8) 4))
                    collect (cons addr count)))
        (file (resolve-fd context fd)))
    (cond (file
           (let ((count (do-read context file iovs)))
             (i32store context size-ptr count))
           +success+)
          (t
           +err-badf+))))

(defun |fd_close| (context fd)
  (let ((file (resolve-fd context fd)))
    (cond (file
           (close-file file)
           +success+)
          (t
           +err-badf+))))

(defun |path_open| (context fd lookup-flags path-buf path-len oflags fs-rights-base fs-rights-inheriting fdflags out-ptr)
  (declare (ignore lookup-flags fs-rights-inheriting))
  (let ((dir (resolve-fd context fd))
        (path (babel:octets-to-string (wasm2cl::wasm-context-memory context)
                                      :start path-buf :end (+ path-buf path-len))))
    (unless (typep dir 'preopened-path)
      (return-from |path_open| +err-badf+))
    (when (logtest oflags +oflags-directory+)
      (error "Not implemented: +oflags-directory+"))
    (when (logtest oflags +oflags-excl+)
      (error "Not implemented: +oflags-excl+"))
    (when (logtest oflags +oflags-trunc+)
      (error "Not implemented: +oflags-trunc"))
    (when (logtest oflags +oflags-creat+)
      (error "Not implemented: +oflags-creat"))
    (when (logtest fdflags +fdflags-append+)
      (error "Not implemented: +fdflags-append+"))
    (let* ((full-path (concatenate 'string (slot-value dir '%path) path))
           (stream (open full-path
                         :direction (cond
                                      ((and (logtest fs-rights-base +rights-fd-read+)
                                            (logtest fs-rights-base +rights-fd-write+))
                                       :io)
                                      ((logtest fs-rights-base +rights-fd-read+)
                                       :input)
                                      ((logtest fs-rights-base +rights-fd-write+)
                                       :output)
                                      (t
                                       (error "Unsupported access rights ~X" fs-rights-base)))
                         :element-type '(unsigned-byte 8)
                         :if-does-not-exist :error
                         :if-exists nil))
           (file (make-instance 'binary-file
                                :path full-path
                                :stream stream))
           (new-fd (vector-push-extend file (slot-value (wasm-context-personality context)
                                                        '%fd-table))))
      (i32store context out-ptr new-fd)
      +success+)))

(defun |proc_exit| (context code)
  (declare (ignore context))
  (throw 'exit code))
