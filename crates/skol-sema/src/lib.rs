//! Name resolution and static type checking for Skol.
//!
//! Turns parsed modules into a single typed [`ir::Program`]. Every rule about
//! what Skol code means (types, coercions, table operation signatures,
//! component composition, closure capture) is enforced here, once, so every
//! backend can trust the IR.

#![forbid(unsafe_code)]

mod builtins;
mod check;

pub use check::{Checker, FsLoader, HostFnSig, ModuleLoader};

/// A compile-time error with its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}: error: {}", self.file, self.line, self.col, self.message)
    }
}

/// Check an entry file (and everything it imports) into a typed program.
pub fn check_file(path: &str, source: &str, loader: &mut dyn ModuleLoader) -> Result<skol_ir::Program, Diagnostic> {
    let mut checker = Checker::new();
    checker.check_entry(path, source, loader)?;
    Ok(checker.into_program())
}
