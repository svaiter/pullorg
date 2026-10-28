;;; gen-entities.el --- Generate src/entities.rs from `org-entities'  -*- lexical-binding: t -*-

;; Usage (from the repository root):
;;
;;   emacs --batch -Q -l org -l tools/gen-entities.el > src/entities.rs
;;
;; Prints the Rust source of `src/entities.rs' on standard output.  The
;; table is sorted by name (byte order) so that `lookup' can use a binary
;; search.  When a name appears several times in `org-entities', only the
;; first occurrence is kept, as `assoc' (used by `org-entity-get') does.

(require 'org-entities)

(defun gen-entities--rust-string (s)
  "Return S as a Rust string literal.
S may also be a character or a symbol: Org 9.8.3 misreads the
\"setminus\" row (its ASCII and Latin-1 fields are written \"\\\" \"\\\"),
which yields a symbol.  Its name is used, so that the table stays
faithful to what Emacs itself exports."
  (cond ((characterp s) (setq s (string s)))
        ((and s (symbolp s)) (setq s (symbol-name s))))
  (concat
   "\""
   (mapconcat
    (lambda (c)
      (cond ((eq c ?\\) "\\\\")
            ((eq c ?\") "\\\"")
            ((eq c ?\n) "\\n")
            ((or (< c 32) (= c 127) (memq c '(#xa0 #x2002 #x2003 #x2009)))
             (format "\\u{%x}" c))
            (t (string c))))
    s "")
   "\""))

(let ((seen (make-hash-table :test #'equal))
      rows)
  (dolist (e org-entities)
    (when (and (consp e) (not (gethash (car e) seen)))
      (puthash (car e) t seen)
      (push e rows)))
  (setq rows (sort rows (lambda (a b)
                          (string< (encode-coding-string (car a) 'utf-8)
                                   (encode-coding-string (car b) 'utf-8)))))
  (princ "//! The Org entities table (`org-entities.el`).
//!
//! This file is generated: do not edit by hand. Regenerate with
//!
//! ```sh
//! emacs --batch -Q -l org -l tools/gen-entities.el > src/entities.rs
//! ```
")
  (princ (format "//!\n//! Generated from Org %s.\n" (org-version)))
  (princ "
/// Look up an entity by name (case-sensitive), in `O(log n)`.
pub(crate) fn lookup(name: &str) -> Option<&'static EntityDef> {
    ENTITIES
        .binary_search_by(|e| e.name.as_bytes().cmp(name.as_bytes()))
        .ok()
        .map(|i| &ENTITIES[i])
}

/// One row of `org-entities`.
#[derive(Debug)]
pub(crate) struct EntityDef {
    pub name: &'static str,
    pub latex: &'static str,
    pub latex_math: bool,
    pub html: &'static str,
    pub ascii: &'static str,
    pub latin1: &'static str,
    pub utf8: &'static str,
}

/// Generated from `org-entities` - see `tools/gen-entities.el`. Sorted by
/// name (byte order).
#[rustfmt::skip]
pub(crate) static ENTITIES: &[EntityDef] = &[
")
  (dolist (e rows)
    (princ (format "    EntityDef { name: %s, latex: %s, latex_math: %s, html: %s, ascii: %s, latin1: %s, utf8: %s },\n"
                   (gen-entities--rust-string (nth 0 e))
                   (gen-entities--rust-string (nth 1 e))
                   (if (nth 2 e) "true" "false")
                   (gen-entities--rust-string (nth 3 e))
                   (gen-entities--rust-string (nth 4 e))
                   (gen-entities--rust-string (nth 5 e))
                   (gen-entities--rust-string (nth 6 e)))))
  (princ "];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_and_unique() {
        for w in ENTITIES.windows(2) {
            assert!(w[0].name.as_bytes() < w[1].name.as_bytes(), \"{} >= {}\", w[0].name, w[1].name);
        }
    }

    #[test]
    fn lookup_known() {
        let a = lookup(\"alpha\").unwrap();
        assert_eq!(a.latex, \"\\\\alpha\");
        assert!(a.latex_math);
        assert_eq!(a.html, \"&alpha;\");
        assert_eq!(a.utf8, \"α\");
        assert_eq!(lookup(\"nbsp\").unwrap().utf8, \"\\u{a0}\");
        assert_eq!(lookup(\"_ \").unwrap().ascii, \" \");
        assert_eq!(lookup(\"_                    \").unwrap().ascii.len(), 20);
        assert!(lookup(\"Alpha\").is_some());
        assert!(lookup(\"alphax\").is_none());
        assert!(lookup(\"\").is_none());
        assert!(lookup(\"_\").is_none());
    }

    #[test]
    fn lookup_all() {
        for e in ENTITIES {
            assert!(std::ptr::eq(lookup(e.name).unwrap(), e));
        }
    }
}
"))
