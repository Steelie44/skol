//! Embed Skol scripts in a Rust application or game.
//!
//! ```no_run
//! use skol::{Engine, Vec2};
//!
//! let mut engine = Engine::new();
//! engine.register("game", "spawn", |kind: String, at: Vec2| -> i64 {
//!     println!("spawning {kind} at {at:?}");
//!     1
//! }).unwrap();
//! engine.load_file("scripts/main.skl").unwrap();
//! engine.call::<(), _>("update", (0.016,)).unwrap();
//! ```
//!
//! A script sees only what the host gives it: the functions you `register`
//! (imported as `from game use spawn`), the built-in prelude and `math`
//! module, and nothing else. File access (`io`) and keyboard input are off
//! unless you turn them on. Every call is bounded by the limits you set, and
//! nothing a script does can panic or crash the host.

#![forbid(unsafe_code)]

mod convert;

use std::fmt;
use std::panic::{self, AssertUnwindSafe};

pub use convert::{HostReturn, IntoArgs, IntoHostFn, SkolType, Vec2, Vec3, Vec4};
pub use skol_ir::Type;
pub use skol_sema::{Diagnostic, FsLoader, ModuleLoader};
pub use skol_vm::{Limits, RuntimeError};

use skol_sema::Checker;
use skol_vm::{HostFn, Vm};

/// Anything that can go wrong embedding a script.
#[derive(Debug, Clone)]
pub enum Error {
    /// The script didn't parse or type-check (includes calls to your API with
    /// the wrong argument types).
    Compile(Diagnostic),
    /// The script failed while running (with a Skol stack trace).
    Runtime(RuntimeError),
    /// The host used the API incorrectly (unknown function, wrong types...).
    Api(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Compile(d) => write!(f, "{d}"),
            Error::Runtime(e) => write!(f, "{e}"),
            Error::Api(m) => write!(f, "skol: {m}"),
        }
    }
}

impl std::error::Error for Error {}

type Output = Box<dyn FnMut(&str)>;
type Input = Box<dyn FnMut() -> Option<String>>;

/// A sandboxed Skol runtime holding one loaded program.
pub struct Engine {
    checker: Checker,
    pending_host_fns: Vec<HostFn>,
    vm: Option<Vm>,
    output: Option<Output>,
    input: Option<Input>,
    loader: Box<dyn ModuleLoader>,
    allow_fs: bool,
    limits: Limits,
    seed: Option<u64>,
}

impl Default for Engine { fn default() -> Self { Engine::new() } }

fn guard<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    panic::catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(Error::Api("internal error in Skol (this is a bug; please report it)".into())))
}

impl Engine {
    /// A new engine with sandboxed defaults: no file access, no keyboard
    /// input, imports resolved next to the entry file, default limits.
    pub fn new() -> Engine {
        Engine {
            checker: Checker::new(),
            pending_host_fns: Vec::new(),
            vm: None,
            output: None,
            input: None,
            loader: Box::new(FsLoader),
            allow_fs: false,
            limits: Limits::default(),
            seed: None,
        }
    }

    fn not_loaded(&self, what: &str) -> Result<(), Error> {
        if self.vm.is_some() { Err(Error::Api(format!("{what} must happen before a script is loaded"))) } else { Ok(()) }
    }

    /// Make a Rust closure callable from scripts as `from <module> use <name>`.
    /// Argument and return types come from the closure's signature; the
    /// checker rejects scripts that call it with the wrong types. Returning
    /// `Err(message)` raises a runtime error in the script.
    pub fn register<Args, F: IntoHostFn<Args>>(&mut self, module: &str, name: &str, f: F) -> Result<&mut Self, Error> {
        self.not_loaded("register")?;
        let (params, ret) = F::signature();
        self.checker.register_host_fn(module, name, params, ret);
        let mut inner = f.into_host_fn();
        let label = format!("{module}.{name}");
        self.pending_host_fns.push(Box::new(move |args| {
            panic::catch_unwind(AssertUnwindSafe(|| inner(args)))
                .unwrap_or_else(|_| Err(format!("host function {label} panicked")))
        }));
        Ok(self)
    }

    /// Allow scripts to use the `io` module (read/write files). Off by default.
    pub fn allow_file_access(&mut self, allow: bool) -> &mut Self {
        self.allow_fs = allow;
        if let Some(vm) = &mut self.vm { vm.allow_fs = allow; }
        self
    }

    /// Bound every call into the script (call depth, loop iterations + calls,
    /// native nesting).
    pub fn set_limits(&mut self, limits: Limits) -> &mut Self {
        self.limits = limits.clone();
        if let Some(vm) = &mut self.vm { vm.limits = limits; }
        self
    }

    /// Where `print` output goes (default: standard output).
    pub fn on_print(&mut self, f: impl FnMut(&str) + 'static) -> &mut Self {
        self.output = Some(Box::new(f));
        self
    }

    /// Where `input()` reads from (default: always end of input).
    pub fn on_input(&mut self, f: impl FnMut() -> Option<String> + 'static) -> &mut Self {
        self.input = Some(Box::new(f));
        self
    }

    /// How `from Name use ...` finds script modules (default: `Name.skl` next to the importing file).
    pub fn set_module_loader(&mut self, loader: impl ModuleLoader + 'static) -> &mut Self {
        self.loader = Box::new(loader);
        self
    }

    /// Fix the random seed (for replays and tests).
    pub fn seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
        if let Some(vm) = &mut self.vm { vm.seed(seed); }
        self
    }

    /// Load, check and initialise a script file. Its top-level code runs now;
    /// `main` is not called automatically.
    pub fn load_file(&mut self, path: &str) -> Result<(), Error> {
        let source = std::fs::read_to_string(path).map_err(|e| Error::Api(format!("couldn't read {path}: {e}")))?;
        self.load_source(path, &source)
    }

    /// Load, check and initialise a script from source text. `name` is used
    /// in error messages and as the base for resolving imports.
    pub fn load_source(&mut self, name: &str, source: &str) -> Result<(), Error> {
        self.not_loaded("load")?;
        guard(|| {
            self.checker.check_entry(name, source, &mut *self.loader).map_err(Error::Compile)?;
            let bc = skol_vm::compile(self.checker.program()).map_err(Error::Api)?;
            let out = self.output.take().unwrap_or_else(|| Box::new(|s: &str| {
                use std::io::Write;
                let _ = std::io::stdout().lock().write_all(s.as_bytes());
            }));
            let mut vm = Vm::with_output(bc, out);
            vm.allow_fs = self.allow_fs;
            vm.limits = self.limits.clone();
            vm.set_input(self.input.take().unwrap_or_else(|| Box::new(|| None)));
            if let Some(s) = self.seed { vm.seed(s); }
            for f in self.pending_host_fns.drain(..) { vm.add_host_fn(f); }
            let inits = vm.bytecode().module_inits.clone();
            self.vm = Some(vm);
            let vm = self.vm.as_mut().expect("vm");
            for init in inits { vm.call(init, vec![]).map_err(Error::Runtime)?; }
            Ok(())
        })
    }

    fn vm(&mut self) -> Result<&mut Vm, Error> {
        self.vm.as_mut().ok_or_else(|| Error::Api("no script is loaded".into()))
    }

    /// Does the loaded script define a top-level function with this name?
    pub fn has_function(&self, name: &str) -> bool { self.checker.entry_function(name).is_some() }

    /// Call a top-level script function, e.g. `engine.call::<(), _>("update", (dt,))`.
    /// The argument and return types must match the function's signature.
    pub fn call<R: SkolType, A: IntoArgs>(&mut self, name: &str, args: A) -> Result<R, Error> {
        let Some((id, params, ret)) = self.checker.entry_function(name) else {
            return Err(Error::Api(format!("the script has no function '{name}'")));
        };
        let given = A::types();
        let prog = self.checker.program();
        if params != given {
            let want: Vec<String> = params.iter().map(|t| prog.type_name(t)).collect();
            let got: Vec<String> = given.iter().map(|t| prog.type_name(t)).collect();
            return Err(Error::Api(format!("{name} takes ({}), but was called with ({})", want.join(", "), got.join(", "))));
        }
        if ret != R::skol_type() {
            return Err(Error::Api(format!("{name} returns {}, not {}", prog.type_name(&ret), prog.type_name(&R::skol_type()))));
        }
        let values = args.into_values();
        guard(|| {
            let v = self.vm()?.call(id, values).map_err(Error::Runtime)?;
            R::from_value(&v).ok_or_else(|| Error::Api(format!("{name} returned an unexpected value")))
        })
    }

    /// Read a top-level `var`/`const` of the loaded script.
    pub fn get<T: SkolType>(&mut self, name: &str) -> Result<T, Error> {
        let Some((id, ty)) = self.checker.entry_global(name) else {
            return Err(Error::Api(format!("the script has no variable '{name}'")));
        };
        if ty != T::skol_type() {
            let prog = self.checker.program();
            return Err(Error::Api(format!("'{name}' has type {}, not {}", prog.type_name(&ty), prog.type_name(&T::skol_type()))));
        }
        let v = self.vm()?.global(id).cloned().unwrap_or(skol_stdlib::Value::Void);
        T::from_value(&v).ok_or_else(|| Error::Api(format!("'{name}' isn't initialised")))
    }

    /// Change a top-level `var` of the loaded script.
    pub fn set<T: SkolType>(&mut self, name: &str, value: T) -> Result<(), Error> {
        let Some((id, ty)) = self.checker.entry_global(name) else {
            return Err(Error::Api(format!("the script has no variable '{name}'")));
        };
        let prog = self.checker.program();
        if !prog.globals[id as usize].mutable {
            return Err(Error::Api(format!("'{name}' is a constant")));
        }
        if ty != T::skol_type() {
            return Err(Error::Api(format!("'{name}' has type {}, not {}", prog.type_name(&ty), prog.type_name(&T::skol_type()))));
        }
        self.vm()?.set_global(id, value.into_value());
        Ok(())
    }
}
