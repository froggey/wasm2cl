(defpackage :wasm2cl
  (:use :cl)
  (:export #:define-wasm-function #:define-wasm-import #:define-wasm-export

           #:i32 #:i64 #:f32 #:f64 #:v128 #:func-ref #:extern-ref

           #:make-wasm-context #:wasm-context
           #:wasm-context-personality #:wasm-context-memory
           #:wasm-context-table #:wasm-context-globals

           #:context #:global #:call-indirect #:select
           #:unreachable
           #:memory-copy #:memory-fill #:memory-grow #:memory-size
           #:i32load #:i32store
           #:i32load8u #:i32load8s #:i32store8
           #:i32load16u #:i32load16s #:i32store16
           #:i32eq #:i32eq.fused
           #:i32ne #:i32ne.fused
           #:i32eqz #:i32eqz.fused
           #:i32leu #:i32leu.fused
           #:i32ltu #:i32ltu.fused
           #:i32geu #:i32geu.fused
           #:i32gtu #:i32gtu.fused
           #:i32les #:i32les.fused
           #:i32lts #:i32lts.fused
           #:i32ges #:i32ges.fused
           #:i32gts #:i32gts.fused
           #:i32add
           #:i32sub
           #:i32mul
           #:i32divu
           #:i32divs
           #:i32remu
           #:i32rems
           #:i32and
           #:i32or
           #:i32xor
           #:i32shl
           #:i32shru
           #:i32shrs
           #:i32rotl
           #:i32rotr
           #:i32clz
           #:i32ctz
           #:i32popcnt
           #:i32wrapi64
           #:i32extend8s
           #:i32extend16s
           #:i64load #:i64store
           #:i64load8u #:i64load8s #:i64store8
           #:i64load16u #:i64load16s #:i64store16
           #:i64load32u #:i64load32s #:i64store32
           #:i64eq #:i64eq.fused
           #:i64ne #:i64ne.fused
           #:i64eqz #:i64eqz.fused
           #:i64leu #:i64leu.fused
           #:i64ltu #:i64ltu.fused
           #:i64geu #:i64geu.fused
           #:i64gtu #:i64gtu.fused
           #:i64les #:i64les.fused
           #:i64lts #:i64lts.fused
           #:i64ges #:i64ges.fused
           #:i64gts #:i64gts.fused
           #:i64add
           #:i64sub
           #:i64mul
           #:i64divu
           #:i64divs
           #:i64remu
           #:i64rems
           #:i64and
           #:i64or
           #:i64xor
           #:i64shl
           #:i64shru
           #:i64shrs
           #:i64rotl
           #:i64rotr
           #:i64clz
           #:i64ctz
           #:i64popcnt
           #:i64extend8s
           #:i64extend16s
           #:i64extend32s
           #:f32const
           #:f32load #:f32store
           #:f32eq #:f32eq.fused
           #:f32ne #:f32ne.fused
           #:f32le #:f32le.fused
           #:f32lt #:f32lt.fused
           #:f32ge #:f32ge.fused
           #:f32gt #:f32gt.fused
           #:f32add
           #:f32sub
           #:f32mul
           #:f32div
           #:f32converti32u
           #:f32converti32s
           #:f32converti64u
           #:f32converti64s
           #:f64const
           #:f64load #:f64store
           #:f64eq #:f64eq.fused
           #:f64ne #:f64ne.fused
           #:f64le #:f64le.fused
           #:f64lt #:f64lt.fused
           #:f64ge #:f64ge.fused
           #:f64gt #:f64gt.fused
           #:f64add
           #:f64sub
           #:f64mul
           #:f64div
           #:f64converti32u
           #:f64converti32s
           #:f64converti64u
           #:f64converti64s))

(in-package :wasm2cl)

(defconstant +wasm-page-size+ #x10000)

(deftype i32 () `(unsigned-byte 32))
(deftype i64 () `(unsigned-byte 64))
(deftype f32 () `single-float)
(deftype f64 () `double-float)

(deftype octet-vector () `(simple-array (unsigned-byte 8) (*)))

(defstruct wasm-context
  (memory (error "memory not supplied") :type octet-vector)
  (table (error "table not supplied") :type simple-vector)
  (globals (error "globals not supplied") :type simple-vector)
  personality)

(defmacro define-wasm-function (name args return-type &body body)
  (declare (ignore return-type))
  `(defun ,name (context . ,(loop for (a) in args collect a))
     (declare (type wasm-context context)
              (ignorable context ,@(loop for (a) in args collect a))
              ,@(loop for (a ty) in args
                      collect `(type ,ty ,a)))
     (block nil
       ,@body)))

(defmacro define-wasm-import (local-name arg-types return-type module name)
  (declare (ignore return-type))
  (let ((package (or (if (string= module "wasi_snapshot_preview1")
                          (find-package :wasm2cl-wasip1))
                     (error "Unknown import module ~S" module))))
    (multiple-value-bind (symbol status)
        (find-symbol (string name) package)
      (unless (eql status :external)
        (error "Unknown import ~S" name))
      (let ((args (loop for nil in arg-types collect (gensym "ARG"))))
        `(defun ,local-name (context . ,args)
           (,symbol context ,@args))))))

(defmacro define-wasm-export (local-name arg-types return-type name)
  (declare (ignore return-type))
  (let ((args (loop for nil in arg-types collect (gensym "ARG"))))
    `(defun ,name (context . ,args)
       (,local-name context ,@args))))

(defmacro f32const (value)
  (let ((tmp (make-array 4 :element-type '(unsigned-byte 8))))
    (declare (dynamic-extent tmp))
    (setf (nibbles:ub32ref/le tmp 0) value)
    (nibbles:ieee-single-ref/le tmp 0)))

(defmacro f64const (value)
  (let ((tmp (make-array 8 :element-type '(unsigned-byte 8))))
    (declare (dynamic-extent tmp))
    (setf (nibbles:ub64ref/le tmp 0) value)
    (nibbles:ieee-double-ref/le tmp 0)))

(declaim (inline global (setf global)))
(defun global (context index)
  (svref (wasm-context-globals context) index))
(defun (setf global) (value context index)
  (setf (svref (wasm-context-globals context) index) value))

(declaim (inline call-indirect))
(defun call-indirect (index context &rest args)
  (apply (svref (wasm-context-table context) index) context args))

(defun sign-extend (value width)
  "Sign extend an value of the specified width."
  (if (logbitp (1- width) value)
      (logior (ash -1 (1- width)) value)
      value))

(define-compiler-macro sign-extend (&whole whole value width)
  (declare (ignorable whole))
  (cond
    #+sbcl
    ((member width '(8 16 32))
     `(sb-vm::sign-extend ,value ,width))
    ((integerp width)
     `(the (signed-byte ,width)
           (let ((value ,value))
             (if (logbitp (1- ,width) value)
                 (logior (ash -1 (1- ,width)) value)
                 value))))
    (t whole)))

(declaim (inline select))
(defun select (lhs rhs test)
  (if test lhs rhs))

(defun unreachable ()
  (error "Unreachable reached!"))

(declaim (inline bool))
(defun bool (value)
  (if value 1 0))

(defun memory-copy (context dst src n)
  (let ((memory (wasm-context-memory context)))
    (replace memory memory
             :start1 dst
             :end1 (+ dst n)
             :start2 src)))

(defun memory-fill (context dst value n)
  (fill (wasm-context-memory context) value
        :start dst
        :end (+ dst n)))

(defun memory-grow (context pages)
  (let ((new-memory (make-array (+ (length (wasm-context-memory context))
                                   (* pages +wasm-page-size+))
                                :element-type '(unsigned-byte 8)))
        (current (length (wasm-context-memory context))))
    (replace new-memory (wasm-context-memory context))
    (setf (wasm-context-memory context) new-memory)
    (truncate current +wasm-page-size+)))

(defun memory-size (context)
  (truncate (length (wasm-context-memory context)) +wasm-page-size+))

(defmacro define-conditional (name args op)
  (let ((fused-name (intern (format nil "~A.FUSED" name)
                            (symbol-package name))))
    `(progn
       (declaim (inline ,name ,fused-name))
       (defun ,fused-name ,args ,op)
       (defun ,name ,args (bool (,fused-name ,@args))))))

(defmacro define-integer-binop (name type signedp op)
  (let ((width (ecase type (i32 32) (i64 64))))
    `(progn
       (declaim (inline ,name))
       (defun ,name (x y)
         (ldb (byte ,width 0)
              (,op ,(if signedp
                        `(sign-extend (the ,type x) ,width)
                        `(the ,type x))
                   ,(if signedp
                        `(sign-extend (the ,type y) ,width)
                        `(the ,type y))))))))

(defmacro define-float-binop (name type op)
  `(progn
     (declaim (inline ,name))
     (defun ,name (x y)
       (,op (the ,type x) (the ,type y)))))

(defun i32load (context address)
  (nibbles:ub32ref/le (wasm-context-memory context) address))

(defun i32load8u (context address)
  (aref (wasm-context-memory context) address))

(defun i32load8s (context address)
  (aref (wasm-context-memory context) address))

(defun i32load16u (context address)
  (nibbles:ub16ref/le (wasm-context-memory context) address))

(defun i32load16s (context address)
  (nibbles:ub16ref/le (wasm-context-memory context) address))

(defun i32store (context address value)
  (setf (nibbles:ub32ref/le (wasm-context-memory context) address) value))

(defun i32store8 (context address value)
  (setf (aref (wasm-context-memory context) address) (ldb (byte 8 0) value)))

(defun i32store16 (context address value)
  (setf (nibbles:ub16ref/le (wasm-context-memory context) address) (ldb (byte 16 0) value)))

(defun i64load (context address)
  (nibbles:ub64ref/le (wasm-context-memory context) address))

(defun i64load8u (context address)
  (aref (wasm-context-memory context) address))

(defun i64load8s (context address)
  (aref (wasm-context-memory context) address))

(defun i64load16u (context address)
  (nibbles:ub16ref/le (wasm-context-memory context) address))

(defun i64load16s (context address)
  (nibbles:ub16ref/le (wasm-context-memory context) address))

(defun i64load32u (context address)
  (nibbles:ub32ref/le (wasm-context-memory context) address))

(defun i64load32s (context address)
  (nibbles:ub32ref/le (wasm-context-memory context) address))

(defun i64store (context address value)
  (setf (nibbles:ub64ref/le (wasm-context-memory context) address) value))

(defun i64store8 (context address value)
  (setf (aref (wasm-context-memory context) address) (ldb (byte 8 0) value)))

(defun i64store16 (context address value)
  (setf (nibbles:ub16ref/le (wasm-context-memory context) address) (ldb (byte 16 0) value)))

(defun i64store32 (context address value)
  (setf (nibbles:ub32ref/le (wasm-context-memory context) address) (ldb (byte 32 0) value)))

(define-conditional i32eqz (x) (eql (the i32 x) 0))
(define-conditional i32eq (x y) (eql (the i32 x) (the i32 y)))
(define-conditional i32ne (x y) (not (eql (the i32 x) (the i32 y))))
(define-conditional i32leu (x y) (<= (the i32 x) (the i32 y)))
(define-conditional i32ltu (x y) (<  (the i32 x) (the i32 y)))
(define-conditional i32geu (x y) (>= (the i32 x) (the i32 y)))
(define-conditional i32gtu (x y) (>  (the i32 x) (the i32 y)))
(define-conditional i32les (x y) (<= (sign-extend (the i32 x) 32) (sign-extend (the i32 y) 32)))
(define-conditional i32lts (x y) (<  (sign-extend (the i32 x) 32) (sign-extend (the i32 y) 32)))
(define-conditional i32ges (x y) (>= (sign-extend (the i32 x) 32) (sign-extend (the i32 y) 32)))
(define-conditional i32gts (x y) (>  (sign-extend (the i32 x) 32) (sign-extend (the i32 y) 32)))

(define-integer-binop i32add i32 nil +)
(define-integer-binop i32sub i32 nil -)
(define-integer-binop i32mul i32 nil *)
(define-integer-binop i32divu i32 nil truncate)
(define-integer-binop i32divs i32 t truncate)
(define-integer-binop i32remu i32 nil rem)
(define-integer-binop i32rems i32 t rem)
(define-integer-binop i32and i32 nil logand)
(define-integer-binop i32or i32 nil logior)
(define-integer-binop i32xor i32 nil logxor)
(define-integer-binop i32shl i32 nil ash)
(define-integer-binop i32shru i32 nil (lambda (x y) (ash x (- y))))
(define-integer-binop i32shrs i32 nil (lambda (x y) (ash (sign-extend x 32) (- y))))

(defun i32rotl (value count)
  (setf count (logand 31 count))
  (logior (ldb (byte count (- 32 count)) value)
          (ash (ldb (byte (- 32 count) 0) value) count)))

(defun i32rotr (value count)
  (setf count (logand 31 count))
  (logior (ash value (- count))
          (ash (ldb (byte count 0) value) (- 32 count))))

(defun i32clz (x)
  (- 32 (integer-length x)))

(defun i32ctz (x)
  (if (zerop x)
      32
      (1- (integer-length (logand x (- x))))))

(defun i32popcnt (x)
  (logcount (the i32 x)))

(declaim (inline i32wrapi64))
(defun i32wrapi64 (x)
  (ldb (byte 32 0) x))

(defun i32extend8s (x)
  (ldb (byte 32 0) (sign-extend x 8)))

(defun i32extend16s (x)
  (ldb (byte 32 0) (sign-extend x 16)))


(define-conditional i64eqz (x) (eql (the i64 x) 0))
(define-conditional i64eq (x y) (eql (the i64 x) (the i64 y)))
(define-conditional i64ne (x y) (not (eql (the i64 x) (the i64 y))))
(define-conditional i64leu (x y) (<= (the i64 x) (the i64 y)))
(define-conditional i64ltu (x y) (<  (the i64 x) (the i64 y)))
(define-conditional i64geu (x y) (>= (the i64 x) (the i64 y)))
(define-conditional i64gtu (x y) (>  (the i64 x) (the i64 y)))
(define-conditional i64les (x y) (<= (sign-extend (the i64 x) 64) (sign-extend (the i64 y) 64)))
(define-conditional i64lts (x y) (<  (sign-extend (the i64 x) 64) (sign-extend (the i64 y) 64)))
(define-conditional i64ges (x y) (>= (sign-extend (the i64 x) 64) (sign-extend (the i64 y) 64)))
(define-conditional i64gts (x y) (>  (sign-extend (the i64 x) 64) (sign-extend (the i64 y) 64)))

(define-integer-binop i64add i64 nil +)
(define-integer-binop i64sub i64 nil -)
(define-integer-binop i64mul i64 nil *)
(define-integer-binop i64divu i64 nil truncate)
(define-integer-binop i64divs i64 t truncate)
(define-integer-binop i64remu i64 nil rem)
(define-integer-binop i64rems i64 t rem)
(define-integer-binop i64and i64 nil logand)
(define-integer-binop i64or i64 nil logior)
(define-integer-binop i64xor i64 nil logxor)
(define-integer-binop i64shl i64 nil ash)
(define-integer-binop i64shru i64 nil (lambda (x y) (ash x (- y))))
(define-integer-binop i64shrs i64 nil (lambda (x y) (ash (sign-extend x 64) (- y))))

(defun i64rotl (value count)
  (setf count (logand 63 count))
  (logior (ldb (byte count (- 64 count)) value)
          (ash (ldb (byte (- 64 count) 0) value) count)))

(defun i64rotr (value count)
  (setf count (logand 63 count))
  (logior (ash value (- count))
          (ash (ldb (byte count 0) value) (- 64 count))))

(defun i64clz (x)
  (- 64 (integer-length x)))

(defun i64ctz (x)
  (if (zerop x)
      64
      (1- (integer-length (logand x (- x))))))

(defun i64popcnt (x)
  (logcount (the i64 x)))

(defun i64extend8s (x)
  (ldb (byte 64 0) (sign-extend x 8)))

(defun i64extend16s (x)
  (ldb (byte 64 0) (sign-extend x 16)))

(defun i64extend32s (x)
  (ldb (byte 64 0) (sign-extend x 32)))

(defun f32load (context address)
  (nibbles:ieee-single-ref/le (wasm-context-memory context) address))

(defun f32store (context address value)
  (setf (nibbles:ieee-single-ref/le (wasm-context-memory context) address) value))

(define-conditional f32eq (x y) (eql (the f32 x) (the f32 y)))
(define-conditional f32ne (x y) (not (eql (the f32 x) (the f32 y))))
(define-conditional f32le (x y) (<= (the f32 x) (the f32 y)))
(define-conditional f32lt (x y) (<  (the f32 x) (the f32 y)))
(define-conditional f32ge (x y) (>= (the f32 x) (the f32 y)))
(define-conditional f32gt (x y) (>  (the f32 x) (the f32 y)))
(define-float-binop f32add f32 +)
(define-float-binop f32sub f32 -)
(define-float-binop f32mul f32 *)
(define-float-binop f32div f32 /)

(defun f32converti32u (x)
  (float (the i32 x) 0.0f0))

(defun f32converti32s (x)
  (float (sign-extend (the i32 x) 32) 0.0f0))

(defun f32converti64u (x)
  (float (the i64 x) 0.0f0))

(defun f32converti64s (x)
  (float (sign-extend (the i64 x) 64) 0.0f0))

(defun f64load (context address)
  (nibbles:ieee-double-ref/le (wasm-context-memory context) address))

(defun f64store (context address value)
  (setf (nibbles:ieee-double-ref/le (wasm-context-memory context) address) value))

(define-conditional f64eq (x y) (eql (the f64 x) (the f64 y)))
(define-conditional f64ne (x y) (not (eql (the f64 x) (the f64 y))))
(define-conditional f64le (x y) (<= (the f64 x) (the f64 y)))
(define-conditional f64lt (x y) (<  (the f64 x) (the f64 y)))
(define-conditional f64ge (x y) (>= (the f64 x) (the f64 y)))
(define-conditional f64gt (x y) (>  (the f64 x) (the f64 y)))
(define-float-binop f64add f64 +)
(define-float-binop f64sub f64 -)
(define-float-binop f64mul f64 *)
(define-float-binop f64div f64 /)

(defun f64converti32u (x)
  (float (the i32 x) 0.0d0))

(defun f64converti32s (x)
  (float (sign-extend (the i32 x) 32) 0.0d0))

(defun f64converti64u (x)
  (float (the i64 x) 0.0d0))

(defun f64converti64s (x)
  (float (sign-extend (the i64 x) 64) 0.0d0))
