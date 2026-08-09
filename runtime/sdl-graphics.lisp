(defpackage :iota-sdl
  (:use :cl :wasm2cl)
  (:export #:*allow-grab*
           #:*enable-audio*
           #:call-with-graphics-support

           #:|_iota_video_init|
           #:|_iota_video_quit|
           #:|_iota_set_video_mode|
           #:|_iota_video_update|
           #:|_iota_poll_event|
           #:|_iota_grab_input|
           #:|_iota_warp_cursor|
           #:|_iota_show_cursor|
           #:|_iota_set_caption|

           #:|_iota_audio_init|
           #:|_iota_audio_request|
           #:|_iota_audio_push|
           #:|_iota_audio_quit|))

(in-package :iota-sdl)

(defparameter *allow-grab* nil
  "When false, attempts to grab input will be ignored.
Some window managers handle this badly and make it impossible to ungrab
input from a frozen or otherwise uncooperative program.")

(defparameter *enable-audio* t)
(defparameter *audio-buffer-margin* 3
  "Keep at least this much audio buffered.
However, large/more buffers = higher latency.")

(defvar *did-sdl-enabled-warning* nil)
(defvar *sdl-enabled* nil)

(defvar *sdl-width*)
(defvar *sdl-height*)
(defvar *sdl-event*)
(defvar *sdl-window*)
(defvar *sdl-renderer*)
(defvar *sdl-texture*)

(defvar *audio-device*)
(defvar *audio-freq*)
(defvar *audio-format*)
(defvar *audio-channels*)
(defvar *audio-samples*)
(defvar *audio-size*)

(defparameter *run-on-main-thread* t)

;; FIXME: This leaves the window open
(defun call-with-graphics-support (fn)
  (sdl2:init :audio)
  (let ((original-terminal-io *terminal-io*)
        (original-standard-output *standard-output*)
        (original-standard-input *standard-input*)
        (original-debug-io *debug-io*)
        (original-trace-output *trace-output*)
        (original-error-output *error-output*))
    (flet ((body ()
             (let ((*sdl-enabled* t)
                   (*sdl-width* 0)
                   (*sdl-height* 0)
                   (*sdl-window* nil)
                   (*sdl-renderer* nil)
                   (*sdl-texture* nil)
                   (*audio-device* nil)
                   (*audio-freq* 0)
                   (*audio-format* 0)
                   (*audio-channels* 0)
                   (*audio-samples* 0)
                   (*audio-size* 0))
               (sdl2:with-sdl-event (event)
                 (setf *sdl-event* event)
                 (unwind-protect
                      (funcall fn)
                   (when *sdl-texture*
                     (sdl2:destroy-texture *sdl-texture*)
                     (setf *sdl-texture* nil))
                   (when *sdl-renderer*
                     (sdl2:destroy-renderer *sdl-renderer*)
                     (setf *sdl-renderer* nil))
                   (when *sdl-window*
                     (sdl2::sdl-set-window-grab *sdl-window* 0)
                     (sdl2:destroy-window *sdl-window*)
                     (setf *sdl-window* nil)))))))
      (unwind-protect
           (if *run-on-main-thread*
               (sdl2:in-main-thread (:no-event t)
                 (let ((*terminal-io* original-terminal-io)
                       (*standard-output* original-standard-output)
                       (*standard-input* original-standard-input)
                       (*debug-io* original-debug-io)
                       (*trace-output* original-trace-output)
                       (*error-output* original-error-output))
                   (body)))
               (body))
        (sdl2:quit)))))

(defun |_iota_video_init| (context)
  (declare (ignore context))
  (cond (*sdl-enabled*
         0)
        (t
         (unless *did-sdl-enabled-warning*
           (warn "Client attempted to initialize SDL but not running within CALL-WITH-GRAPHICS-SUPPORT")
           (setf *did-sdl-enabled-warning* t))
         1)))

(defun |_iota_video_quit| (context)
  (declare (ignore context))
  0)

(defun |_iota_set_video_mode| (context width height)
  (declare (ignore context))
  (cond (*sdl-enabled*
         (when *sdl-texture*
                (sdl2:destroy-texture *sdl-texture*)
                (setf *sdl-texture* nil))
         (when *sdl-renderer*
           (sdl2:destroy-renderer *sdl-renderer*)
           (setf *sdl-renderer* nil))
         (when *sdl-window*
           (sdl2::sdl-set-window-grab *sdl-window* 0)
           (sdl2:destroy-window *sdl-window*)
           (setf *sdl-window* nil))
         (setf *sdl-width* width
               *sdl-height* height)
         (setf *sdl-window*
               (sdl2:create-window :w width :h height :flags '(:shown)))
         (setf *sdl-renderer*
               (sdl2:create-renderer *sdl-window*))
         (setf *sdl-texture*
               (sdl2:create-texture *sdl-renderer*
                                    :argb8888
                                    :streaming
                                    width height))
         0)
        (t
         1)))

(defun |_iota_video_update| (context buf)
  (when (and (not (zerop buf)) *sdl-texture*)
    (let ((memory (wasm-context-memory context)))
      (declare (type (simple-array (unsigned-byte 8) (*)) memory))
      (cffi:with-pointer-to-vector-data (ptr memory)
        (sdl2:update-texture *sdl-texture*
                             nil
                             (cffi:inc-pointer ptr buf)
                             (* *sdl-width* 4))))
    (sdl2:render-clear *sdl-renderer*)
    (sdl2:render-copy *sdl-renderer* *sdl-texture*)
    (sdl2:render-present *sdl-renderer*))
  0)

(defun translate-key-code (key-code)
  (case key-code
    (:unknown 0)
    (:return 13)
    (:escape 27)
    (:backspace 8)
    (:tab 9)
    (:space 32)
    (:exclaim 33)
    (:quotedbl 34)
    (:hash 35)
    (:percent 37)
    (:dollar 36)
    (:ampersand 38)
    (:quote 39)
    (:leftparen 40)
    (:rightparen 41)
    (:asterisk 42)
    (:plus 43)
    (:comma 44)
    (:minus 45)
    (:period 46)
    (:slash 47)
    (:|0| 48)
    (:|1| 49)
    (:|2| 50)
    (:|3| 51)
    (:|4| 52)
    (:|5| 53)
    (:|6| 54)
    (:|7| 55)
    (:|8| 56)
    (:|9| 57)
    (:colon 58)
    (:semicolon 59)
    (:less 60)
    (:equals 61)
    (:greater 62)
    (:question 63)
    (:at 64)
    (:leftbracket 91)
    (:backslash 92)
    (:rightbracket 93)
    (:caret 94)
    (:underscore 95)
    (:backquote 96)
    (:a 97)
    (:b 98)
    (:c 99)
    (:d 100)
    (:e 101)
    (:f 102)
    (:g 103)
    (:h 104)
    (:i 105)
    (:j 106)
    (:k 107)
    (:l 108)
    (:m 109)
    (:n 110)
    (:o 111)
    (:p 112)
    (:q 113)
    (:r 114)
    (:s 115)
    (:t 116)
    (:u 117)
    (:v 118)
    (:w 119)
    (:x 120)
    (:y 121)
    (:z 122)
    (:delete 127)
    (:kp-0 256)
    (:kp-1 257)
    (:kp-2 258)
    (:kp-3 259)
    (:kp-4 260)
    (:kp-5 261)
    (:kp-6 262)
    (:kp-7 263)
    (:kp-8 264)
    (:kp-9 265)
    (:kp-period 266)
    (:kp-divide 267)
    (:kp-multiply 268)
    (:kp-minus 269)
    (:kp-plus 270)
    (:kp-enter 271)
    (:kp-equals 272)
    (:up 273)
    (:down 274)
    (:right 275)
    (:left 276)
    (:insert 277)
    (:home 278)
    (:end 279)
    (:pageup 280)
    (:pagedown 281)
    (:f1 282)
    (:f2 283)
    (:f3 284)
    (:f4 285)
    (:f5 286)
    (:f6 287)
    (:f7 288)
    (:f8 289)
    (:f9 290)
    (:f10 291)
    (:f11 292)
    (:f12 293)
    (:f13 294)
    (:f14 295)
    (:f15 296)
    (:capslock 301)
    (:scrolllock 302)
    (:rshift 303)
    (:lshift 304)
    (:rctrl 305)
    (:lctrl 306)
    (:ralt 307)
    (:lalt 308)
    (:lgui 311)
    (:rgui 312)
    (:mode 313)
    (:help 315)
    (:printscreen 316)
    (:sysreq 317)
    (:pause 318)
    (:menu 319)
    (:power 320)
    (:undo 322)
    (t 0)))

(defun translate-keysym-event (event)
  (let ((keysym (plus-c:c-ref event sdl2-ffi:sdl-event :key :keysym)))
    (values (sdl2:scancode-value keysym)
            (sdl2:mod-value keysym)
            (translate-key-code
             (autowrap:enum-key 'sdl2-ffi:sdl-key-code (sdl2:sym-value keysym))))))

;; See SDL-1.2.15/src/video/iota/SDL_iotaevents.c for event definitions.
(defun |_iota_poll_event| (context buf)
  (loop
     until (zerop (sdl2:next-event *sdl-event* :poll))
     do
       (case (sdl2:get-event-type *sdl-event*)
         (:quit
          (i32store context buf 0)
          (return 1))
         (:keydown
          (multiple-value-bind (scancode mod key)
              (translate-keysym-event *sdl-event*)
            (i32store context buf 1)
            (i32store context (+ buf 4) scancode)
            (i32store context (+ buf 8) mod)
            (i32store context (+ buf 12) key))
          (return 1))
         (:keyup
          (multiple-value-bind (scancode mod key)
              (translate-keysym-event *sdl-event*)
            (i32store context buf 2)
            (i32store context (+ buf 4) scancode)
            (i32store context (+ buf 8) mod)
            (i32store context (+ buf 12) key))
          (return 1))
         (:mousemotion
          (i32store context buf 3)
          (i32store context (+ buf 4) (ldb (byte 32 0)
                                           (plus-c:c-ref *sdl-event*
                                                         sdl2-ffi:sdl-event
                                                         :motion :xrel)))
          (i32store context (+ buf 8) (ldb (byte 32 0)
                                           (plus-c:c-ref *sdl-event*
                                                         sdl2-ffi:sdl-event
                                                         :motion :yrel)))
          (return 1))
         (:mousebuttondown
          (i32store context buf 4)
          (i32store context (+ buf 4) (plus-c:c-ref *sdl-event*
                                                    sdl2-ffi:sdl-event
                                                    :button :button))
          (return 1))
         (:mousebuttonup
          (i32store context buf 5)
          (i32store context (+ buf 4) (plus-c:c-ref *sdl-event*
                                                    sdl2-ffi:sdl-event
                                                    :button :button))
          (return 1))
         (:textinput) ; ignore this
         (:windowevent
          (let ((evt-type (autowrap:enum-key 'sdl2-ffi:sdl-window-event-id
                                             (plus-c:c-ref *sdl-event*
                                                           sdl2-ffi:sdl-event
                                                           :window :event))))
            (when (member evt-type '(:focus-gained :focus-lost))
              (i32store context buf 6)
              (i32store context (+ buf 4) (if (eql evt-type :focus-gained)
                                              1
                                              0))
              (i32store context (+ buf 8) 2) ; SDL_APPINPUTFOCUS
              (return 1))))
         (t
          (format t "Ignoring SDL event ~S~%" (sdl2:get-event-type *sdl-event*))))
     finally (return 0)))

(defun |_iota_grab_input| (context mode)
  (declare (ignore context))
  (cond ((and *allow-grab*
              *sdl-window*)
         (sdl2::sdl-set-window-grab *sdl-window* mode))
        (t
         (if (zerop mode)
             0
             1))))

(defun |_iota_warp_cursor| (context x y)
  (declare (ignore context))
  (when *sdl-window*
    (sdl2:warp-mouse-in-window *sdl-window* x y))
  0)

(defconstant +sdl-query+ -1)
(defconstant +sdl-disable+ 0)
(defconstant +sdl-enable+ 1)

(defun |_iota_show_cursor| (context toggle)
  (declare (ignore context))
  (case (sign-extend toggle 32)
    (#.+sdl-enable+ (sdl2:show-cursor))
    (#.+sdl-disable+ (sdl2:hide-cursor)))
  (if (sdl2:show-cursor-p) +sdl-enable+ +sdl-disable+))

(defun |_iota_set_caption| (context title icon)
  (declare (ignore icon))
  (sdl2:set-window-title *sdl-window* (read-c-string context title)))

(defun |_iota_audio_init| (context freq format channels samples size)
  (declare (ignore context))
  (setf *audio-freq* freq
        *audio-format* format
        *audio-channels* channels
        *audio-samples* samples
        *audio-size* size)
  (setf *audio-device* nil)
  (when (not (and *enable-audio* *sdl-enabled*))
    ;; Immediate success when disabled.
    (return-from |_iota_audio_init| 0))
  (let ((dev (plus-c:c-let ((desired sdl2-ffi:sdl-audio-spec :calloc t))
               (setf (plus-c:c-ref desired sdl2-ffi:sdl-audio-spec :freq) freq
                     (plus-c:c-ref desired sdl2-ffi:sdl-audio-spec :format) format
                     (plus-c:c-ref desired sdl2-ffi:sdl-audio-spec :channels) channels
                     (plus-c:c-ref desired sdl2-ffi:sdl-audio-spec :samples) samples)
               (sdl2-ffi.functions:sdl-open-audio-device nil 0 desired nil 0))))
    (when (zerop dev)
      (return-from |_iota_audio_init| 1))
    (sdl2-ffi.functions:sdl-pause-audio-device dev 0)
    (setf *audio-device* dev)
    0))

(defun |_iota_audio_request| (context)
  (declare (ignore context))
  (if *audio-device*
      (let ((watermark (* *audio-buffer-margin* *audio-size*))
            (queued (sdl2-ffi.functions:sdl-get-queued-audio-size *audio-device*)))
        (max 0 (- watermark queued)))
      0))

(defun |_iota_audio_push| (context stream len)
  (when *audio-device*
    (cffi:with-pointer-to-vector-data (base (wasm-context-memory context))
      (sdl2-ffi.functions:sdl-queue-audio
       *audio-device* (cffi:inc-pointer base stream) len)))
  nil)

(defun |_iota_audio_quit| (context)
  (declare (ignore context))
  (when *audio-device*
    (sdl2-ffi.functions:sdl-close-audio-device *audio-device*)
    (setf *audio-device* nil))
  nil)
