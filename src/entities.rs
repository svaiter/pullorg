//! The Org entities table (`org-entities.el`).

/// Look up an entity by name. Returns
/// `(name, latex, latex_math, html, ascii, latin1, utf8)`.
pub(crate) fn lookup(name: &str) -> Option<&'static EntityDef> {
    ENTITIES.iter().find(|e| e.name == name)
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

/// Generated from `org-entities` — see `tools/gen-entities.el`.
pub(crate) static ENTITIES: &[EntityDef] = &[];
