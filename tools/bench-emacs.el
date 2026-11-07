;;; bench-emacs.el --- Time org-element-parse-buffer on a directory of Org files  -*- lexical-binding: t -*-

;; Emacs side of the pullorg/Emacs comparison.  See tools/bench-vs-emacs.sh
;; and benches/README.md.
;;
;; Usage:
;;
;;   emacs --batch -Q -l org -l tools/bench-emacs.el \
;;         -f pullorg-bench-batch DIR OUT.tsv SAMPLES WARMUP
;;
;; Every DIR/*.org is parsed with `org-element-parse-buffer' at granularity
;; `element' and `object' (the default), with the element cache off.  The
;; measurement rule is the one of benches/vs_emacs.rs: one calibration run
;; picks the iteration count that makes a sample last at least 10 ms;
;; WARMUP samples are discarded; SAMPLES samples are timed; the report gives
;; the median, minimum and maximum wall time per iteration, garbage
;; collection included, and the mean GC time per iteration (GC pauses fall
;; in a few samples only, so their median is usually zero).  OUT.tsv gets
;; one line per file and level:
;;
;;   name  bytes  level  median_s  min_s  max_s  gc_s
;;
;; with level `elements' or `full', and a first comment line describing the
;; Emacs and Org that ran.

;;; Code:

(require 'org)
(require 'org-element)
(require 'cl-lib)

(defconst pullorg-bench--target-sample 0.010
  "Minimum duration of one sample, in seconds.")

(defun pullorg-bench--compilation (sym)
  "How the function SYM is compiled: native, byte or interpreted."
  (let ((f (indirect-function sym)))
    (cond ((or (and (fboundp 'native-comp-function-p) (native-comp-function-p f))
               (and (fboundp 'subr-native-elisp-p) (subr-native-elisp-p f)))
           "native")
          ((byte-code-function-p f) "byte")
          (t "interpreted"))))

(defun pullorg-bench--sample (granularity iters)
  "Run ITERS parses at GRANULARITY; return (SECONDS . GC-SECONDS) per parse."
  (let ((gc0 gc-elapsed)
        (t0 (float-time)))
    (dotimes (_ iters)
      (org-element-parse-buffer granularity))
    (cons (/ (- (float-time) t0) iters)
          (/ (- gc-elapsed gc0) iters))))

(defun pullorg-bench--median (xs)
  (let* ((v (vconcat (sort (copy-sequence xs) #'<)))
         (n (length v)))
    (if (cl-oddp n)
        (aref v (/ n 2))
      (/ (+ (aref v (1- (/ n 2))) (aref v (/ n 2))) 2.0))))

(defun pullorg-bench--measure (granularity samples warmup)
  "Time the current buffer at GRANULARITY; return (MEDIAN MIN MAX GC)."
  (let* ((once (max 1e-6 (car (pullorg-bench--sample granularity 1))))
         (iters (max 1 (ceiling (/ pullorg-bench--target-sample once))))
         times gcs)
    (dotimes (i (+ warmup samples))
      (let ((s (pullorg-bench--sample granularity iters)))
        (when (>= i warmup)
          (push (car s) times)
          (push (cdr s) gcs))))
    (list (pullorg-bench--median times)
          (apply #'min times)
          (apply #'max times)
          (/ (apply #'+ gcs) (length gcs)))))

(defun pullorg-bench-batch ()
  "Entry point; see the commentary."
  (pcase-let ((`(,dir ,out ,samples ,warmup) command-line-args-left))
    (setq command-line-args-left nil)
    (let ((samples (string-to-number samples))
          (warmup (string-to-number warmup))
          (org-inhibit-startup t)
          (org-element-use-cache nil)
          (lines (list (format "# emacs %s, org %s, org-element %s\n"
                               emacs-version (org-version)
                               (pullorg-bench--compilation
                                'org-element--current-element)))))
      (dolist (file (directory-files dir t "\\.org\\'"))
        (let ((name (file-name-base file)))
          (with-temp-buffer
            (insert-file-contents file)
            (let ((bytes (1- (position-bytes (point-max)))))
              (org-mode)
              (pcase-dolist (`(,level . ,granularity)
                             '(("elements" . element) ("full" . object)))
                (pcase-let ((`(,median ,min ,max ,gc)
                             (pullorg-bench--measure granularity samples warmup)))
                  (message "emacs   %-20s %-8s %10.3f ms" name level (* median 1e3))
                  (push (format "%s\t%d\t%s\t%.9f\t%.9f\t%.9f\t%.9f\n"
                                name bytes level median min max gc)
                        lines)))))))
      (with-temp-file out
        (apply #'insert (nreverse lines))))))

;;; bench-emacs.el ends here
