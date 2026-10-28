;;; dump.el --- Dump the org-element tree of an Org file as JSON  -*- lexical-binding: t -*-

;; Part of the pullorg conformance harness.  See tests/README.md.
;;
;; Usage:
;;
;;   emacs --batch -Q -l org -l tests/emacs/dump.el \
;;         -f pullorg-dump-batch IN.org OUT.json [IN2.org OUT2.json ...]
;;
;; Output shape (one node per element/object):
;;
;;   {"type": "org-data", "children": [...]}                       ; root
;;   {"type": "paragraph", "begin": 0, "end": 12, "props": {...}, "children": [...]}
;;   {"type": "plain-text", "value": "..."}
;;
;; Conventions:
;;
;; - `begin'/`end' are 0-based byte offsets in the UTF-8 source.
;; - Element `end' excludes trailing blank lines: from `:end', skip back over
;;   whitespace, then move to the beginning of the next line (or EOB).
;; - Object `end' is `:end' minus `:post-blank' (trailing whitespace
;;   excluded).  The post-blank whitespace of an object is re-materialized
;;   as a plain-text node right after it (and merged with adjacent
;;   plain-text), so that the text of a container is complete, as in the
;;   pullorg event stream where that whitespace is part of the next
;;   `Text' event.
;; - Timestamps stored in properties (planning, clock) are emitted without
;;   positions; objects inside secondary strings that are emitted as node
;;   arrays (headline `title', item `tag') keep their positions.
;; - Citation (and citation-reference) `prefix'/`suffix' secondary strings
;;   are emitted as their raw text (`org-element-interpret-data').
;; - Affiliated keywords are read back from the buffer lines between
;;   `:begin' and `:post-affiliated' (i.e. exactly the lines Emacs attached
;;   to the element) and emitted in `props.affiliated' as
;;   {"key", "value", "optional"} objects sorted by key, value, optional.
;;   The key is upper-cased and translated through
;;   `org-element-keyword-translation-alist'; the value is the trimmed raw
;;   text (CAPTION is not parsed); `optional' is the bracketed value of
;;   dual keywords (CAPTION, RESULTS), null otherwise.
;; - Booleans are true/false, absent values are null, symbols are strings,
;;   the headline priority is an integer, as Emacs stores it (character
;;   code for a letter, the number itself for a numeric priority).

;;; Code:

(require 'org)
(require 'org-element)
(require 'json)
(require 'cl-lib)

(defun pullorg-dump--bytes (pos)
  "0-based byte offset of buffer position POS."
  (1- (position-bytes pos)))

(defun pullorg-dump--str (v)
  "Convert V (string, symbol or nil) to a JSON string or null."
  (cond ((null v) :null)
        ((stringp v) (substring-no-properties v))
        ((symbolp v) (symbol-name v))
        ((numberp v) (number-to-string v))
        (t (format "%s" v))))

(defun pullorg-dump--num (v)
  (if (numberp v) v :null))

(defun pullorg-dump--bool (v)
  (if v t :false))

(defun pullorg-dump--strings (l)
  (vconcat (mapcar #'pullorg-dump--str l)))

(defun pullorg-dump--raw-secondary (s)
  "Raw text of secondary string S, or null."
  (if s
      (substring-no-properties (org-element-interpret-data s))
    :null))

(defun pullorg-dump--element-end (el)
  (save-excursion
    (let ((begin (org-element-property :begin el)))
      (goto-char (org-element-property :end el))
      (skip-chars-backward " \t\n\r" begin)
      (unless (bolp) (forward-line 1))
      (pullorg-dump--bytes (min (point) (point-max))))))

(defun pullorg-dump--object-p (node)
  (memq (org-element-type node) org-element-all-objects))

(defun pullorg-dump--affiliated (el)
  "Affiliated keywords of EL read back from the buffer."
  (let ((begin (org-element-property :begin el))
        (post (org-element-property :post-affiliated el))
        (case-fold-search t)
        out)
    (when (and begin post (< begin post))
      (save-excursion
        (goto-char begin)
        (while (and (< (point) post) (looking-at org-element--affiliated-re))
          (let* ((raw (upcase (match-string-no-properties 1)))
                 (key (or (cdr (assoc raw org-element-keyword-translation-alist))
                          raw))
                 (opt (and (member key org-element-dual-keywords)
                           (match-string-no-properties 2)))
                 (vbeg (match-end 0))
                 (vend (save-excursion (end-of-line) (point))))
            (push `((key . ,key)
                    (value . ,(org-trim (buffer-substring-no-properties vbeg vend)))
                    (optional . ,(if opt opt :null)))
                  out))
          (forward-line 1))))
    (when out
      (vconcat
       (sort out
             (lambda (a b)
               (let ((ka (list (alist-get 'key a) (alist-get 'value a)
                               (let ((o (alist-get 'optional a))) (if (stringp o) o ""))))
                     (kb (list (alist-get 'key b) (alist-get 'value b)
                               (let ((o (alist-get 'optional b))) (if (stringp o) o "")))))
                 (cl-loop for x in ka for y in kb
                          if (string< x y) return t
                          if (string< y x) return nil
                          finally return nil))))))))

(defun pullorg-dump--timestamp-props (ts)
  (let ((p (lambda (k) (org-element-property k ts))))
    `((type . ,(pullorg-dump--str (funcall p :type)))
      (raw-value . ,(pullorg-dump--str (funcall p :raw-value)))
      ,@(mapcar (lambda (k)
                  (cons (intern (substring (symbol-name k) 1))
                        (pullorg-dump--num (funcall p k))))
                '(:year-start :month-start :day-start :hour-start :minute-start
                  :year-end :month-end :day-end :hour-end :minute-end))
      (repeater-type . ,(pullorg-dump--str (funcall p :repeater-type)))
      (repeater-value . ,(pullorg-dump--num (funcall p :repeater-value)))
      (repeater-unit . ,(pullorg-dump--str (funcall p :repeater-unit)))
      (warning-type . ,(pullorg-dump--str (funcall p :warning-type)))
      (warning-value . ,(pullorg-dump--num (funcall p :warning-value)))
      (warning-unit . ,(pullorg-dump--str (funcall p :warning-unit))))))

(defun pullorg-dump--embedded-ts (ts)
  "Timestamp TS stored in a property: a node without positions."
  (if ts
      `((type . "timestamp")
        (props . ,(pullorg-dump--timestamp-props ts))
        (children . []))
    :null))

(defun pullorg-dump--headline-props (el)
  (let ((p (lambda (k) (org-element-property k el))))
    `((level . ,(funcall p :level))
      (todo-keyword . ,(pullorg-dump--str (funcall p :todo-keyword)))
      (todo-type . ,(pullorg-dump--str (funcall p :todo-type)))
      (priority . ,(or (funcall p :priority) :null))
      (tags . ,(pullorg-dump--strings (funcall p :tags)))
      (commentedp . ,(pullorg-dump--bool (funcall p :commentedp)))
      (archivedp . ,(pullorg-dump--bool (funcall p :archivedp)))
      (footnote-section-p . ,(pullorg-dump--bool (funcall p :footnote-section-p)))
      (raw-value . ,(pullorg-dump--str (funcall p :raw-value)))
      (title . ,(pullorg-dump--contents (funcall p :title))))))

(defun pullorg-dump--props (node)
  "Type-specific properties of NODE, as an alist."
  (let* ((type (org-element-type node))
         (p (lambda (k) (org-element-property k node)))
         (s (lambda (k) (pullorg-dump--str (funcall p k)))))
    (pcase type
      ((or 'headline 'inlinetask) (pullorg-dump--headline-props node))
      ('item
       `((bullet . ,(funcall s :bullet))
         (checkbox . ,(funcall s :checkbox))
         (counter . ,(pullorg-dump--num (funcall p :counter)))
         (tag . ,(let ((tag (funcall p :tag)))
                   (if tag (pullorg-dump--contents tag) :null)))))
      ('plain-list `((type . ,(funcall s :type))))
      ('table
       `((type . ,(funcall s :type))
         (tblfm . ,(pullorg-dump--strings (funcall p :tblfm)))
         (value . ,(funcall s :value))))
      ('table-row `((type . ,(funcall s :type))))
      ('src-block
       `((language . ,(funcall s :language))
         (switches . ,(funcall s :switches))
         (parameters . ,(funcall s :parameters))
         (value . ,(funcall s :value))))
      ('example-block
       `((switches . ,(funcall s :switches))
         (value . ,(funcall s :value))))
      ('export-block
       `((type . ,(funcall s :type))
         (value . ,(funcall s :value))))
      ('comment-block `((value . ,(funcall s :value))))
      ('special-block
       `((type . ,(funcall s :type))
         (parameters . ,(funcall s :parameters))))
      ('drawer `((drawer-name . ,(funcall s :drawer-name))))
      ('dynamic-block
       `((block-name . ,(funcall s :block-name))
         (arguments . ,(funcall s :arguments))))
      ('footnote-definition `((label . ,(funcall s :label))))
      ('keyword
       `((key . ,(funcall s :key))
         (value . ,(funcall s :value))))
      ((or 'comment 'fixed-width 'latex-environment 'diary-sexp
           'latex-fragment 'code 'verbatim 'statistics-cookie 'target
           'radio-target)
       `((value . ,(funcall s :value))))
      ((or 'babel-call 'inline-babel-call)
       `((call . ,(funcall s :call))
         (inside-header . ,(funcall s :inside-header))
         (arguments . ,(funcall s :arguments))
         (end-header . ,(funcall s :end-header))))
      ('clock
       `((status . ,(funcall s :status))
         (duration . ,(funcall s :duration))
         (value . ,(pullorg-dump--embedded-ts (funcall p :value)))))
      ('planning
       `((closed . ,(pullorg-dump--embedded-ts (funcall p :closed)))
         (scheduled . ,(pullorg-dump--embedded-ts (funcall p :scheduled)))
         (deadline . ,(pullorg-dump--embedded-ts (funcall p :deadline)))))
      ('node-property
       `((key . ,(funcall s :key))
         (value . ,(funcall s :value))))
      ('link
       `((type . ,(funcall s :type))
         (path . ,(funcall s :path))
         (format . ,(funcall s :format))
         (raw-link . ,(funcall s :raw-link))
         (search-option . ,(funcall s :search-option))
         (application . ,(funcall s :application))))
      ('timestamp (pullorg-dump--timestamp-props node))
      ('entity
       `((name . ,(funcall s :name))
         (use-brackets-p . ,(pullorg-dump--bool (funcall p :use-brackets-p)))))
      ('export-snippet
       `((back-end . ,(funcall s :back-end))
         (value . ,(funcall s :value))))
      ('inline-src-block
       `((language . ,(funcall s :language))
         (parameters . ,(funcall s :parameters))
         (value . ,(funcall s :value))))
      ('macro
       `((key . ,(funcall s :key))
         (args . ,(pullorg-dump--strings (funcall p :args)))))
      ('footnote-reference
       `((label . ,(funcall s :label))
         (type . ,(funcall s :type))))
      ((or 'subscript 'superscript)
       `((use-brackets-p . ,(pullorg-dump--bool (funcall p :use-brackets-p)))))
      ('citation
       `((style . ,(funcall s :style))
         (prefix . ,(pullorg-dump--raw-secondary (funcall p :prefix)))
         (suffix . ,(pullorg-dump--raw-secondary (funcall p :suffix)))))
      ('citation-reference
       `((key . ,(funcall s :key))
         (prefix . ,(pullorg-dump--raw-secondary (funcall p :prefix)))
         (suffix . ,(pullorg-dump--raw-secondary (funcall p :suffix)))))
      (_ nil))))

(defun pullorg-dump--node (node)
  "Convert NODE (an element, object or string) to a JSON alist.
Return a list of nodes: an object with post-blank whitespace is followed
by a plain-text node holding that whitespace."
  (if (stringp node)
      (list `((type . "plain-text")
              (value . ,(substring-no-properties node))))
    (let* ((type (org-element-type node))
           (object (pullorg-dump--object-p node))
           (begin (org-element-property :begin node))
           (end (org-element-property :end node))
           (post-blank (or (org-element-property :post-blank node) 0))
           (props (pullorg-dump--props node))
           (aff (and (not object) (pullorg-dump--affiliated node)))
           (props (if aff (append props `((affiliated . ,aff))) props))
           (json `((type . ,(symbol-name type))
                   (begin . ,(pullorg-dump--bytes begin))
                   (end . ,(if object
                               (pullorg-dump--bytes (- end post-blank))
                             (pullorg-dump--element-end node)))
                   (props . ,(or props (make-hash-table)))
                   (children . ,(pullorg-dump--contents
                                 (org-element-contents node))))))
      (if (and object (> post-blank 0))
          (list json
                `((type . "plain-text")
                  (value . ,(buffer-substring-no-properties (- end post-blank) end))))
        (list json)))))

(defun pullorg-dump--contents (contents)
  "Convert a list of nodes CONTENTS to a JSON vector, merging plain text."
  (let (out)
    (dolist (c contents)
      (dolist (j (pullorg-dump--node c))
        (if (and out
                 (equal (alist-get 'type j) "plain-text")
                 (equal (alist-get 'type (car out)) "plain-text"))
            (setcar out `((type . "plain-text")
                          (value . ,(concat (alist-get 'value (car out))
                                            (alist-get 'value j)))))
          (push j out))))
    (vconcat (nreverse out))))

(defun pullorg-dump-file (in out)
  "Parse Org file IN and write its JSON tree to OUT."
  (let ((tree
         (with-temp-buffer
           (let ((coding-system-for-read 'utf-8-unix))
             (insert-file-contents in))
           (let ((org-inhibit-startup nil))
             (org-mode))
           (let ((data (org-element-parse-buffer)))
             `((type . "org-data")
               (children . ,(pullorg-dump--contents
                             (org-element-contents data))))))))
    (with-temp-buffer
      (insert (decode-coding-string
               (json-serialize tree :null-object :null :false-object :false)
               'utf-8))
      (let ((json-encoding-pretty-print t)
            (json-encoding-default-indentation "  "))
        (json-pretty-print-buffer))
      (goto-char (point-max))
      (insert "\n")
      (let ((coding-system-for-write 'utf-8-unix))
        (write-region (point-min) (point-max) out nil 'silent)))))

(defun pullorg-dump-batch ()
  "Batch entry point: remaining command-line args are IN OUT pairs."
  (unless (and command-line-args-left
               (zerop (% (length command-line-args-left) 2)))
    (error "Usage: -f pullorg-dump-batch IN.org OUT.json [IN OUT ...]"))
  (while command-line-args-left
    (let ((in (pop command-line-args-left))
          (out (pop command-line-args-left)))
      (pullorg-dump-file in out))))

(provide 'pullorg-dump)
;;; dump.el ends here
