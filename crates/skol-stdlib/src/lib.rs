//! Skol runtime library: the value model, tables, signals and built-in
//! functions. Backend-agnostic — the VM uses it today, and an AOT backend can
//! link the same code later.

#![forbid(unsafe_code)]

pub mod builtins;
pub mod format;
pub mod table;
pub mod value;

pub use table::{Key, Table, TableData};

/// Largest string a script may build (concatenation, repeat, file reads).
/// Keeps a runaway script from exhausting memory and aborting the process.
pub const MAX_STRING_BYTES: usize = 64 * 1024 * 1024;
pub use value::{Closure, Instance, SignalObj, Value, VecN};

/// What the runtime needs from its host (the VM): names for printing,
/// somewhere to write output, and a random-number state.
pub trait Host: skol_ir::TypeNames {
    fn field_names(&self, class: u32) -> &[String];
    fn func_name(&self, func: u32) -> &str;
    fn write_out(&mut self, text: &str);
    fn rng_state(&mut self) -> &mut u64;
    fn elapsed_secs(&self) -> f64;
    /// Whether scripts may touch the file system (the `io` module).
    fn fs_allowed(&self) -> bool { true }
    /// Read one line of input without its line ending; `None` at end of input.
    fn read_line(&mut self) -> Result<Option<String>, String> {
        use std::io::{BufRead, Write};
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let n = std::io::stdin().lock().read_line(&mut line).map_err(|e| format!("couldn't read input: {e}"))?;
        if n == 0 { return Ok(None); }
        while line.ends_with('\n') || line.ends_with('\r') { line.pop(); }
        Ok(Some(line))
    }
    /// Call a function registered by the embedding application.
    fn call_host(&mut self, id: u32, args: &[Value]) -> Result<Value, String> {
        let _ = args;
        Err(format!("host function #{id} is not available"))
    }
}
