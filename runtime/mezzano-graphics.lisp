;;; Implementation of the `iota-sdl` API implemented using Mezzano

(defpackage :iota-sdl
  (:use :cl :wasm2cl)
  (:export #:call-with-graphics-support

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

(defvar *graphics-enabled* nil)

(defvar *graphics-width*)
(defvar *graphics-height*)
(defvar *graphics-fifo*)
(defvar *graphics-window*)
(defvar *graphics-frame*)

(defparameter *audio-buffer-margin* 2)

(defvar *audio-sink*)
(defvar *audio-freq*)
(defvar *audio-format*)
(defvar *audio-channels*)
(defvar *audio-samples*)
(defvar *audio-size*)
(defvar *audio-src-output*)

(defun call-with-graphics-support (fn)
  (let ((*graphics-enabled* t)
        (*graphics-width* 0)
        (*graphics-height* 0)
        (*graphics-fifo* (mezzano.supervisor:make-fifo 50))
        (*graphics-window* nil)
        (*graphics-frame* nil)
        (*audio-sink* nil)
        (*audio-freq* 0)
        (*audio-format* 0)
        (*audio-channels* 0)
        (*audio-samples* 0)
        (*audio-size* 0)
        (*audio-src-output* nil))
    (unwind-protect
         (funcall fn)
      (when *audio-sink*
        (mezzano.driver.sound:flush-sink *audio-sink*))
      (when *graphics-window*
        (mezzano.gui.compositor:close-window *graphics-window*)))))

(defun |_iota_video_init| (context)
  (declare (ignore context))
  (if *graphics-enabled*
      0
      1))

(defun |_iota_video_quit| (context)
  (declare (ignore context))
  ;; TODO: Clean up any window here.
  0)

(defun compute-window-size (width height)
  ;; Make a fake frame to get the frame size.
  (multiple-value-bind (left right top bottom)
      (mezzano.gui.widgets:frame-size (make-instance 'mezzano.gui.widgets:frame))
    (values (+ left (max 32 width) right)
            (+ top (max 32 height) bottom))))

(defun |_iota_set_video_mode| (context width height)
  (declare (ignore context))
  (cond (*graphics-enabled*
         (multiple-value-bind (real-width real-height)
             (compute-window-size width height)
           (when *graphics-window*
             (mezzano.gui.compositor:close-window *graphics-window*)
             (setf *graphics-window* nil))
           (setf *graphics-window* (mezzano.gui.compositor:make-window
                                    *graphics-fifo*
                                    real-width
                                    real-height)
                 *graphics-width* width
                 *graphics-height* height)
           (setf *graphics-frame*
                 (make-instance 'mezzano.gui.widgets:frame
                                :framebuffer (mezzano.gui.compositor:window-buffer *graphics-window*)
                                :title "wasm2cl host"
                                :close-button-p t
                                :damage-function (mezzano.gui.widgets:default-damage-function *graphics-window*)))
           (mezzano.gui.widgets:draw-frame *graphics-frame*)
         0))
        (t
         1)))

(defun |_iota_video_update| (context buf)
  (declare (optimize (speed 3) (safety 0))
           (type (unsigned-byte 32) buf))
  (when (not (zerop buf))
    (multiple-value-bind (left right top bottom)
        (mezzano.gui.widgets:frame-size *graphics-frame*)
      (declare (ignore right bottom)
               (type fixnum left top))
      (loop
         with framebuffer = (mezzano.gui.compositor:window-buffer *graphics-window*)
         with pixels = (the (simple-array (unsigned-byte 32) (* *))
                            (mezzano.gui:surface-pixels framebuffer))
         with win-width fixnum = (mezzano.gui:surface-width framebuffer)
         with g-width fixnum = *graphics-width*
         with g-height fixnum = *graphics-height*
         for y fixnum below g-height
         do
           (loop
              for x fixnum below g-width
              do (setf (row-major-aref pixels (the fixnum (+ left (the fixnum (+ x (the fixnum (* (the fixnum (+ top y)) win-width)))))))
                       (logior #xFF000000
                               (i32load context (the fixnum (+ buf (the fixnum (* (the fixnum (+ x (the fixnum (* y g-width)))) 4)))))))))
      (mezzano.gui.compositor:damage-window *graphics-window*
                                            left top
                                            *graphics-width* *graphics-height*)))
  0)

(defun translate-sdl-keysym (translated-key original-key)
  (case original-key
    (#\Newline        13)
    (#\F1            282)
    (#\F2            283)
    (#\F3            284)
    (#\F4            285)
    (#\F5            286)
    (#\F6            287)
    (#\F7            288)
    (#\F8            289)
    (#\F9            290)
    (#\F10           291)
    (#\F11           292)
    (#\F12           293)
    (#\F13           294)
    (#\F14           295)
    (#\F15           296)
    (#\Insert        277)
    (#\Delete        127)
    (#\Home          278)
    (#\End           279)
    (#\Page-Up       280)
    (#\Page-Down     281)
    (#\Left-Arrow    276)
    (#\Right-Arrow   275)
    (#\Up-Arrow      273)
    (#\Down-Arrow    274)
    (#\Menu          319)
    (#\Print-Screen  317)
    (#\Pause          19)
    (#\Break         318)
    (#\Caps-Lock     301)
    (#\Left-Shift    305)
    (#\Right-Shift   304)
    (#\Left-Control  306)
    (#\Right-Control 305)
    (#\Left-Meta     308)
    (#\Right-Meta    307)
    (#\Left-Super    312)
    (#\Right-Super   311)
    (#\KP-0          256)
    (#\KP-1          257)
    (#\KP-2          258)
    (#\KP-3          259)
    (#\KP-4          260)
    (#\KP-5          261)
    (#\KP-6          262)
    (#\KP-7          263)
    (#\KP-8          264)
    (#\KP-9          265)
    (#\KP-Period     266)
    (#\KP-Divide     267)
    (#\KP-Multiply   268)
    (#\KP-Minus      269)
    (#\KP-Plus       270)
    (#\KP-Enter      271)
    (t (char-code translated-key))))

(defun |_iota_poll_event| (context buf)
  (loop
     (let ((evt (mezzano.supervisor:fifo-pop *graphics-fifo* nil)))
       (when (not evt) (return 0))
       (typecase evt
         ((or mezzano.gui.compositor:window-close-event
              mezzano.gui.compositor:quit-event)
          (i32store context buf 0) ; quit
          (return 1))
         (mezzano.gui.compositor:mouse-event
          (handler-case
              (progn
                (mezzano.gui.widgets:frame-mouse-event *graphics-frame* evt)
                ;; FIXME: This needs to send multiple events...
                (cond ((not (zerop (mezzano.gui.compositor:mouse-button-change evt)))
                       ;; Find the changed button. Hope only one changed.
                       (let ((button (loop
                                        for i from 0
                                        until (logbitp i (mezzano.gui.compositor:mouse-button-change evt))
                                        finally (return i))))
                         (i32store context
                                   buf
                                   (if (logbitp button (mezzano.gui.compositor:mouse-button-state evt))
                                       4 ; mouse-button-down
                                       5)) ; mouse-button-up
                          (i32store context (+ buf 4) (1+ button))))
                      (t
                       ;; No changes, send a motion event.
                       (i32store context buf 3) ; mouse-motion
                       (i32store context (+ buf 4) (ldb (byte 32 0) (mezzano.gui.compositor:mouse-x-motion evt)))
                       (i32store context (+ buf 8) (ldb (byte 32 0) (mezzano.gui.compositor:mouse-y-motion evt)))))
                (return 1))
            (mezzano.gui.widgets:close-button-clicked ()
              (i32store context buf 0) ; quit
              (return 1))))
         (mezzano.gui.compositor:key-event
          (i32store context
                    buf
                    (if (mezzano.gui.compositor:key-releasep evt)
                         2 ; key-up
                         1)) ; key-down
          (i32store context (+ buf 4) (char-code (mezzano.gui.compositor:key-scancode evt)))
          (i32store context (+ buf 8) 0) ;; FIXME
          (i32store context (+ buf 12)
                    (translate-sdl-keysym (mezzano.gui.compositor:key-key evt)
                                          (mezzano.gui.compositor:key-scancode evt)))
          (return 1))
         (mezzano.gui.compositor:window-activation-event
          (setf (mezzano.gui.widgets:activep *graphics-frame*) (mezzano.gui.compositor:state evt))
          (mezzano.gui.widgets:draw-frame *graphics-frame*)
          (i32store context buf 6) ; Activation event
          (i32store context
                    (+ buf 4)
                    (if (mezzano.gui.compositor:state evt)
                        1
                        0))
          (i32store context (+ buf 8) #x7)))))) ; mouse, input, and active.

(defun set-input-grab (grabp)
  (multiple-value-bind (left right top bottom)
      (mezzano.gui.widgets:frame-size *graphics-frame*)
    (declare (ignore right bottom))
    ;; Clamp the grab region to the interior of the frame, not the whole window.
    (mezzano.gui.compositor:grab-cursor *graphics-window* grabp
                                        left top
                                        *graphics-width* *graphics-height*)))

(defun |_iota_grab_input| (context mode)
  (declare (ignore context))
  (set-input-grab (not (zerop mode)))
  mode)

(defun |_iota_warp_cursor| (context x y)
  (declare (ignore context x y))
  )

(defun |_iota_show_cursor| (context toggle)
  (declare (ignore context))
  (set-input-grab (zerop toggle))
  (mezzano.gui.compositor:set-window-data
   *graphics-window*
   :cursor (if (not (eql toggle 0))
               :default
               :none)))

(defun |_iota_set_caption| (context title icon)
  (declare (ignore icon))
  (let ((title-text (read-c-string context title)))
    (setf (mezzano.gui.widgets:frame-title *graphics-frame*) title-text)
    (mezzano.gui.widgets:draw-frame *graphics-frame*)
    (mezzano.gui.compositor:set-window-data *graphics-window* :title title-text)))

(defconstant +output-audio-frequency+ 44100)

(defun resample-buffer (input-rate output-rate input start end output)
  "Sample-rate-convert stereo s16le PCM via linear interpolation.
INPUT is a byte vector of interleaved s16le stereo samples.
START and END are byte offsets (must be 4-byte aligned).
OUTPUT is a pre-allocated byte vector for the result.
Returns the number of bytes written to OUTPUT."
  (let* ((ratio (/ input-rate output-rate))
         (n-input-frames (/ (- end start) 4))
         (n-output-frames (max 1 (round (* n-input-frames (/ output-rate input-rate))))))
    (dotimes (i n-output-frames)
      (let* ((pos (* i ratio))
             (idx (floor pos))
             (frac (- pos idx))
             (off-in (+ start (* idx 4)))
             (off-out (* i 4)))
        (flet ((interp-channel (offset)
                 (let* ((s0 (/ (mezzano.extensions:sb16ref/le input (+ off-in offset)) 32768.0))
                        (s1 (if (< (+ idx 1) n-input-frames)
                                (/ (mezzano.extensions:sb16ref/le input (+ off-in offset 4)) 32768.0)
                                s0)))
                   (+ (* s0 (- 1.0 frac)) (* s1 frac)))))
          (setf (mezzano.extensions:sb16ref/le output off-out)
                (round (* (interp-channel 0) 32768.0)))
          (setf (mezzano.extensions:sb16ref/le output (+ off-out 2))
                (round (* (interp-channel 2) 32768.0))))))
    (* n-output-frames 4)))

(defconstant +audio-s16le+ #x8010)

(defun |_iota_audio_init| (context freq format channels samples size)
  (declare (ignore context))
  (when (/= format +audio-s16le+)
    (format t "~&[iota-audio] unsupported format #x~X~%" format)
    (return-from |_iota_audio_init| 1))
  (when (/= channels 2)
    (format t "~&[iota-audio] unsupported channel count ~A~%" channels)
    (return-from |_iota_audio_init| 1))
  (setf *audio-freq* freq
        *audio-format* format
        *audio-channels* channels
        *audio-samples* samples
        *audio-size* size)
  (handler-case
      (setf *audio-sink*
            (mezzano.driver.sound:make-sound-output-sink
             :buffer-duration 0.1
             :format :pcm-s16le))
    (error (c)
      (format t "~&[iota-audio] failed to create sink: ~A~%" c)
      (setf *audio-sink* nil)
      (return-from |_iota_audio_init| 1)))
  (setf *audio-src-output* (make-array (* size (ceiling +output-audio-frequency+ freq))
                                       :element-type '(unsigned-byte 8)))
  0)

(defun |_iota_audio_request| (context)
  (declare (ignore context))
  (if *audio-sink*
      (let ((watermark (* *audio-buffer-margin* *audio-size*))
            (buffered (/ (* (mezzano.driver.sound:sink-buffered-frames *audio-sink*)
                            ;; bytes per sample
                            2)
                         ;; Since we're upscaling from 11khz to 44khz
                         4)))
        (max 0 (- watermark buffered)))
      0))

(defun |_iota_audio_push| (context stream len)
  (when (and *audio-sink* (plusp len))
    (let ((memory (wasm-context-memory context))
          (end (+ stream len)))
      (declare (type (simple-array (unsigned-byte 8) (*)) memory))
      (cond ((= *audio-freq* +output-audio-frequency+)
             ;; Direct path, no resampling.
             (mezzano.driver.sound:output-sound memory *audio-sink*
                                                :start stream :end end))
            (t
             ;; Need to resample.
             (let ((count (resample-buffer *audio-freq* +output-audio-frequency+ memory stream end *audio-src-output*)))
               (mezzano.driver.sound:output-sound
                *audio-src-output*
                *audio-sink*
                :end count))))))
  nil)

(defun |_iota_audio_quit| (context)
  (declare (ignore context))
  (when *audio-sink*
    (mezzano.driver.sound:flush-sink *audio-sink*)
    (setf *audio-sink* nil
          *audio-src-output* nil))
  nil)
