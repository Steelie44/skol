use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use skol_ir::{self as ir, BinOp, Cmp, Expr, ExprKind, FuncId, Place, Stmt, StmtKind, TableOp, Type, UnOp, UpvalSource};
use skol_syntax::ast;
use skol_syntax::Span;

use crate::builtins::{self, MathSym, Resolved};
use crate::Diagnostic;

type R<T> = Result<T, Diagnostic>;

// ------------------------------------------------------------------ loading

/// Finds imported modules. `importer` is the path of the importing file.
pub trait ModuleLoader {
    /// Return `None` if the module doesn't exist, otherwise its canonical
    /// path and source text (or an I/O error message).
    fn load(&mut self, importer: &str, name: &str) -> Option<Result<(String, String), String>>;
    /// Where this loader looks, for "can't find module" errors.
    fn searched(&self) -> String { "next to this file".into() }
}

/// Loads `Name.skl` from the importing file's directory, then from each
/// library folder in order.
#[derive(Debug, Clone, Default)]
pub struct FsLoader {
    pub search: Vec<std::path::PathBuf>,
}

impl FsLoader {
    /// Only the importing file's own folder (the embedding default).
    pub fn new() -> FsLoader { FsLoader { search: Vec::new() } }

    /// The command-line search order: the importing file's folder, every
    /// folder in the `SKOL_PATH` environment variable, then the standard
    /// library: a `lib` folder next to the `skol` executable or up to two
    /// levels above it (so a build in `target/release` finds the repo's `lib`).
    pub fn standard() -> FsLoader {
        let mut search = Vec::new();
        if let Some(p) = std::env::var_os("SKOL_PATH") {
            search.extend(std::env::split_paths(&p).filter(|d| !d.as_os_str().is_empty()));
        }
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            let mut dir = Some(exe_dir.as_path());
            for _ in 0..3 {
                let Some(d) = dir else { break };
                let lib = d.join("lib");
                if lib.join("skol.lib").is_file() { search.push(lib); break; }
                dir = d.parent();
            }
        }
        FsLoader { search }
    }

    pub fn with_path(mut self, dir: impl Into<std::path::PathBuf>) -> FsLoader { self.search.push(dir.into()); self }

    /// Human-readable list of the places searched, for error messages.
    pub fn describe(&self) -> String {
        if self.search.is_empty() { return "next to this file".into(); }
        let dirs: Vec<String> = self.search.iter().map(|d| d.to_string_lossy().into_owned()).collect();
        format!("next to this file and in {}", dirs.join(", "))
    }
}

impl ModuleLoader for FsLoader {
    fn load(&mut self, importer: &str, name: &str) -> Option<Result<(String, String), String>> {
        // Module names are plain identifiers, so they can't climb out of a folder.
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') { return None; }
        let dir = Path::new(importer).parent().map(Path::to_path_buf).unwrap_or_default();
        let file = format!("{name}.skl");
        let path = std::iter::once(dir).chain(self.search.iter().cloned())
            .map(|d| d.join(&file))
            .find(|p| p.is_file())?;
        let shown = path.to_string_lossy().into_owned();
        Some(std::fs::read_to_string(&path).map(|s| (shown, s)).map_err(|e| e.to_string()))
    }

    fn searched(&self) -> String { self.describe() }
}

// ------------------------------------------------------------------ state

#[derive(Debug, Clone)]
enum Symbol {
    Global(ir::GlobalId),
    Func(FuncId),
    Class(ir::ClassId),
    Comp(ir::CompId),
    Signal(ir::SignalId),
    MathFn(&'static str),
    MathConst(f64),
    /// A function registered by the embedding application.
    Host(u32),
}

/// Signature of a function the embedding application exposes to scripts.
#[derive(Debug, Clone)]
pub struct HostFnSig { pub name: String, pub params: Vec<Type>, pub ret: Type }

#[derive(Debug, Clone, PartialEq)]
enum ModState { Loading, Done }

#[derive(Debug, Clone)]
struct Module {
    path: String,
    file: ir::FileId,
    scope: HashMap<String, Symbol>,
    state: ModState,
}

#[derive(Debug, Clone)]
struct CompSrc { decl: Rc<ast::ComponentDecl>, module: usize }

#[derive(Debug, Clone)]
struct MethodImpl { func: FuncId, decl: Rc<ast::FnDecl>, module: usize }

#[derive(Debug, Clone)]
struct FieldSrc { default: Option<ast::Expr>, module: usize, span: Span }

#[derive(Debug, Clone)]
struct ClassSrc {
    decl: Rc<ast::ClassDecl>,
    module: usize,
    methods: Vec<MethodImpl>,
    fields: Vec<FieldSrc>,
    init: Option<FuncId>,
}

#[derive(Debug, Clone)]
struct FnCtx {
    func: ir::Function,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    local_mut: Vec<bool>,
    upval_mut: Vec<bool>,
    /// Lambdas can see enclosing locals; named functions can't.
    is_lambda: bool,
    /// How many loops enclose the current statement (for break/continue).
    loops: u32,
}

enum VarRef { Local(ir::LocalId, Type, bool), Upval(u16, Type, bool), Global(ir::GlobalId), Sym(Symbol), Prelude(&'static str) }

/// The type checker. Keeps its state between calls so a REPL can feed it
/// one chunk at a time.
#[derive(Clone)]
pub struct Checker {
    prog: ir::Program,
    modules: Vec<Module>,
    by_path: HashMap<String, usize>,
    comp_src: Vec<CompSrc>,
    class_src: Vec<ClassSrc>,
    fstack: Vec<FnCtx>,
    module: usize,
    file: ir::FileId,
    span: Span,
    repl: Option<usize>,
    /// Host modules: module name -> (function name -> host id).
    host_modules: HashMap<String, HashMap<String, u32>>,
    host_fns: Vec<HostFnSig>,
    entry: Option<usize>,
    /// `const` globals whose value is known at compile time; references
    /// compile to the literal (so `x / CELL` becomes a constant operation).
    consts: HashMap<ir::GlobalId, ExprKind>,
}

/// A compile-time constant value.
#[derive(Debug, Clone)]
enum ConstVal { I(i64), F(f64), S(String), B(bool) }

impl Default for Checker { fn default() -> Self { Self::new() } }

impl Checker {
    pub fn new() -> Self {
        Checker {
            prog: ir::Program::default(),
            modules: Vec::new(),
            by_path: HashMap::new(),
            comp_src: Vec::new(),
            class_src: Vec::new(),
            fstack: Vec::new(),
            module: 0,
            file: 0,
            span: Span::default(),
            repl: None,
            host_modules: HashMap::new(),
            host_fns: Vec::new(),
            entry: None,
            consts: HashMap::new(),
        }
    }

    /// Expose a host function to scripts as `from <module> use <name>`.
    /// Returns its host id (the index the VM's host function table must use).
    pub fn register_host_fn(&mut self, module: &str, name: &str, params: Vec<Type>, ret: Type) -> u32 {
        let id = self.host_fns.len() as u32;
        self.host_fns.push(HostFnSig { name: format!("{module}.{name}"), params, ret });
        self.host_modules.entry(module.to_string()).or_default().insert(name.to_string(), id);
        id
    }

    /// A top-level function of the entry module: (id, parameter types, return type).
    pub fn entry_function(&self, name: &str) -> Option<(FuncId, Vec<Type>, Type)> {
        match self.modules.get(self.entry?)?.scope.get(name)? {
            Symbol::Func(id) => {
                let f = &self.prog.functions[*id as usize];
                Some((*id, f.params.clone(), f.ret.clone()))
            }
            _ => None,
        }
    }

    /// A top-level variable or constant of the entry module: (id, type).
    pub fn entry_global(&self, name: &str) -> Option<(ir::GlobalId, Type)> {
        match self.modules.get(self.entry?)?.scope.get(name)? {
            Symbol::Global(g) => Some((*g, self.prog.globals[*g as usize].ty.clone())),
            _ => None,
        }
    }

    pub fn program(&self) -> &ir::Program { &self.prog }
    pub fn into_program(self) -> ir::Program { self.prog }

    /// Check the entry module; sets `program().main` if it defines `fn main()`.
    pub fn check_entry(&mut self, path: &str, source: &str, loader: &mut dyn ModuleLoader) -> R<()> {
        let m = self.check_module(path, source, loader)?;
        self.entry = Some(m);
        if let Some(Symbol::Func(id)) = self.modules[m].scope.get("main").cloned() {
            let f = &self.prog.functions[id as usize];
            if !f.params.is_empty() {
                return Err(self.diag_at(f.file, f.span, "fn main() must not take parameters".into()));
            }
            if f.ret != Type::Void && f.ret != Type::Int {
                return Err(self.diag_at(f.file, f.span, "fn main() must return void or i".into()));
            }
            self.prog.main = Some(id);
        }
        Ok(())
    }

    /// Check one REPL input. Names persist between chunks and may be
    /// redefined. Top-level expressions with a value are printed. Returns the
    /// chunk's init function. On error the checker is left unchanged.
    pub fn check_repl_chunk(&mut self, source: &str, loader: &mut dyn ModuleLoader) -> R<FuncId> {
        let snapshot = self.clone();
        let result = self.repl_chunk_inner(source, loader);
        if result.is_err() { *self = snapshot; }
        result
    }

    fn repl_chunk_inner(&mut self, source: &str, loader: &mut dyn ModuleLoader) -> R<FuncId> {
        let m = match self.repl {
            Some(m) => m,
            None => {
                let file = self.add_file("<repl>", source);
                let m = self.modules.len();
                self.modules.push(Module { path: "<repl>".into(), file, scope: HashMap::new(), state: ModState::Done });
                self.repl = Some(m);
                m
            }
        };
        // Each chunk gets its own file entry so error positions are right.
        let file = self.add_file("<repl>", source);
        self.modules[m].file = file;
        let program = skol_syntax::parse(source)
            .map_err(|e| Diagnostic { message: e.message, file: "<repl>".into(), line: e.line, col: e.col })?;
        self.module = m;
        self.file = file;
        self.process_items(m, &program.items, loader, "<repl>")
    }

    fn add_file(&mut self, path: &str, source: &str) -> ir::FileId {
        self.prog.files.push(ir::SourceFile { path: path.into(), source: source.into() });
        (self.prog.files.len() - 1) as ir::FileId
    }

    fn check_module(&mut self, path: &str, source: &str, loader: &mut dyn ModuleLoader) -> R<usize> {
        if let Some(&m) = self.by_path.get(path) {
            if self.modules[m].state == ModState::Loading {
                return Err(self.diag(format!("circular import of '{path}'")));
            }
            return Ok(m);
        }
        let program = skol_syntax::parse(source)
            .map_err(|e| Diagnostic { message: e.message, file: path.into(), line: e.line, col: e.col })?;
        let file = self.add_file(path, source);
        let m = self.modules.len();
        self.modules.push(Module { path: path.into(), file, scope: HashMap::new(), state: ModState::Loading });
        self.by_path.insert(path.into(), m);

        let saved = (self.module, self.file, self.span);
        self.module = m;
        self.file = file;
        let name = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let init = self.process_items(m, &program.items, loader, &name)?;
        self.prog.module_inits.push(init);
        self.modules[m].state = ModState::Done;
        (self.module, self.file, self.span) = saved;
        Ok(m)
    }

    // ------------------------------------------------------------- passes

    fn process_items(&mut self, m: usize, items: &[ast::Item], loader: &mut dyn ModuleLoader, mod_name: &str) -> R<FuncId> {
        // 1. imports
        for item in items {
            if let ast::ItemKind::Import(imp) = &item.kind {
                self.span = item.span;
                self.import(m, imp, loader)?;
            }
        }

        // 2. declare names
        let mut new_comps = Vec::new();
        let mut new_classes = Vec::new();
        let mut new_fns = Vec::new();
        let mut new_globals = Vec::new();
        let mut new_signals = Vec::new();
        for item in items {
            self.span = item.span;
            match &item.kind {
                ast::ItemKind::Import(_) | ast::ItemKind::Stmt(_) => {}
                ast::ItemKind::Component(c) => {
                    let id = self.prog.comps.len() as ir::CompId;
                    self.prog.comps.push(ir::Comp { name: c.name.clone(), fields: vec![], methods: HashMap::new() });
                    self.comp_src.push(CompSrc { decl: c.clone(), module: m });
                    self.define(m, &c.name, Symbol::Comp(id))?;
                    new_comps.push(id);
                }
                ast::ItemKind::Class(c) => {
                    let id = self.prog.classes.len() as ir::ClassId;
                    self.prog.classes.push(ir::Class {
                        name: c.name.clone(), comps: vec![], fields: vec![], field_index: HashMap::new(),
                        methods: HashMap::new(), ctor: 0, default_ctor: 0,
                    });
                    self.class_src.push(ClassSrc { decl: c.clone(), module: m, methods: vec![], fields: vec![], init: None });
                    self.define(m, &c.name, Symbol::Class(id))?;
                    new_classes.push(id);
                }
                ast::ItemKind::Signal(s) => {
                    let id = self.prog.signals.len() as ir::SignalId;
                    let global = self.prog.globals.len() as ir::GlobalId;
                    self.prog.globals.push(ir::Global { name: s.name.clone(), ty: Type::Signal(id), mutable: false });
                    self.prog.signals.push(ir::Signal { name: s.name.clone(), params: s.params.clone(), global });
                    self.define(m, &s.name, Symbol::Signal(id))?;
                    new_signals.push(id);
                }
                ast::ItemKind::Fn(f) => {
                    let id = self.alloc_func(&f.name, ir::FuncKind::Function, f.span);
                    self.define(m, &f.name, Symbol::Func(id))?;
                    new_fns.push((id, f.clone()));
                }
                ast::ItemKind::Binding(b) => {
                    let id = self.prog.globals.len() as ir::GlobalId;
                    self.prog.globals.push(ir::Global { name: b.name.clone(), ty: Type::Void, mutable: b.mutable });
                    self.define(m, &b.name, Symbol::Global(id))?;
                    new_globals.push((id, item.span));
                }
            }
        }

        // 3. resolve signatures and layouts
        for &c in &new_comps { self.resolve_comp(c)?; }
        for (id, f) in &new_fns {
            self.span = f.span;
            let (params, ret) = self.resolve_sig(&f.params, &f.ret_ty)?;
            let func = &mut self.prog.functions[*id as usize];
            func.params = params;
            func.ret = ret;
        }
        let mut gi = 0;
        for item in items {
            if let ast::ItemKind::Binding(b) = &item.kind {
                self.span = item.span;
                let ty = self.resolve_type(&b.ty)?;
                if ty == Type::Void { return Err(self.diag(format!("'{}' can't have type void", b.name))); }
                let gid = new_globals[gi].0;
                if !b.mutable {
                    if let Some(k) = self.fold_const(&b.init).and_then(|v| const_as(v, &ty)) {
                        self.consts.insert(gid, k);
                    }
                }
                self.prog.globals[gid as usize].ty = ty;
                gi += 1;
            }
        }
        for &c in &new_classes { self.layout_class(c)?; }

        // 4. bodies
        for (id, f) in &new_fns { self.check_fn_body(*id, f, None, m)?; }
        for &c in &new_comps { self.check_comp_standalone(c)?; }
        for &c in &new_classes { self.check_class_bodies(c)?; }

        // 5. module initialiser: signals, then globals and statements in order
        let init_span = items.first().map(|i| i.span).unwrap_or_default();
        let init = self.alloc_func(&format!("<init {mod_name}>"), ir::FuncKind::ModuleInit, init_span);
        self.push_fn(init, false);
        let mut body = Vec::new();
        for &s in &new_signals {
            let sig = &self.prog.signals[s as usize];
            body.push(Stmt {
                kind: StmtKind::Assign { place: Place::Global(sig.global), op: None, value: Expr { span: Span::default(), kind: ExprKind::NewSignal(s), ty: Type::Signal(s) } },
                span: init_span,
            });
        }
        let mut gi = 0;
        for item in items {
            self.span = item.span;
            match &item.kind {
                ast::ItemKind::Binding(b) => {
                    let gid = new_globals[gi].0;
                    gi += 1;
                    let ty = self.prog.globals[gid as usize].ty.clone();
                    let value = self.expr_as(&b.init, &ty, &format!("initializer of '{}'", b.name))?;
                    body.push(Stmt { kind: StmtKind::Assign { place: Place::Global(gid), op: None, value }, span: item.span });
                }
                ast::ItemKind::Stmt(s) => {
                    if let Some(st) = self.stmt(s)? {
                        let st = if self.repl == Some(m) { echo(st) } else { st };
                        body.push(st);
                    }
                }
                _ => {}
            }
        }
        self.pop_fn(init, body);
        Ok(init)
    }

    fn define(&mut self, m: usize, name: &str, sym: Symbol) -> R<()> {
        if self.repl != Some(m) && self.modules[m].scope.contains_key(name) {
            return Err(self.diag(format!("'{name}' is already defined in this module")));
        }
        self.modules[m].scope.insert(name.to_string(), sym);
        Ok(())
    }

    fn import(&mut self, m: usize, imp: &ast::ImportDecl, loader: &mut dyn ModuleLoader) -> R<()> {
        if let Some(fns) = self.host_modules.get(&imp.module).cloned() {
            for name in &imp.names {
                let Some(&id) = fns.get(name) else {
                    return Err(self.diag(format!("module '{}' has no '{}'", imp.module, name)));
                };
                self.define(m, name, Symbol::Host(id))?;
            }
            return Ok(());
        }
        if builtins::is_builtin_module(&imp.module) {
            for name in &imp.names {
                let sym = match builtins::module_symbol(&imp.module, name) {
                    Some(MathSym::Func(f)) => Symbol::MathFn(f),
                    Some(MathSym::Const(c)) => Symbol::MathConst(c),
                    None => return Err(self.diag(format!(
                        "module '{}' has no '{name}' (it has: {})", imp.module, builtins::module_names(&imp.module).join(", ")))),
                };
                self.define(m, name, sym)?;
            }
            return Ok(());
        }
        let importer = self.modules[m].path.clone();
        match loader.load(&importer, &imp.module) {
            Some(Ok((path, source))) => {
                let saved_span = self.span;
                let child = self.check_module(&path, &source, loader)?;
                self.span = saved_span;
                for name in &imp.names {
                    let sym = self.modules[child].scope.get(name).cloned()
                        .ok_or_else(|| self.diag(format!("module '{}' has no '{}'", imp.module, name)))?;
                    self.define(m, name, sym)?;
                }
                Ok(())
            }
            Some(Err(e)) => Err(self.diag(format!("couldn't read module '{}': {e}", imp.module))),
            None => Err(self.diag(format!("can't find module '{}' (looked for {}.skl {})", imp.module, imp.module, loader.searched()))),
        }
    }

    fn alloc_func(&mut self, name: &str, kind: ir::FuncKind, span: Span) -> FuncId {
        self.prog.functions.push(ir::Function {
            name: name.to_string(), kind, params: vec![], ret: Type::Void, locals: vec![], upvalues: vec![],
            body: vec![], file: self.file, span,
        });
        (self.prog.functions.len() - 1) as FuncId
    }

    fn resolve_sig(&mut self, params: &[ast::Param], ret: &ast::TypeExpr) -> R<(Vec<Type>, Type)> {
        if params.len() > 254 {
            return Err(self.diag(format!("too many parameters ({}); the limit is 254", params.len())));
        }
        let mut ps = Vec::new();
        for p in params {
            let t = self.resolve_type(&p.ty)?;
            if t == Type::Void { return Err(self.diag(format!("parameter '{}' can't have type void", p.name))); }
            ps.push(t);
        }
        Ok((ps, self.resolve_type(ret)?))
    }

    fn with_module<T>(&mut self, m: usize, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = (self.module, self.file);
        self.module = m;
        self.file = self.modules[m].file;
        let r = f(self);
        (self.module, self.file) = saved;
        r
    }

    fn resolve_comp(&mut self, c: ir::CompId) -> R<()> {
        let src = self.comp_src[c as usize].clone();
        self.with_module(src.module, |this| {
            let mut fields: Vec<(String, Type)> = Vec::new();
            for f in &src.decl.fields {
                this.span = f.span;
                if fields.iter().any(|(n, _)| *n == f.name) {
                    return Err(this.diag(format!("field '{}' is declared twice in component {}", f.name, src.decl.name)));
                }
                let ty = this.resolve_type(&f.ty)?;
                if f.default.is_none() && !this.has_zero(&ty) {
                    return Err(this.diag(format!("field '{}' of type {} needs a default value", f.name, this.tn(&ty))));
                }
                fields.push((f.name.clone(), ty));
            }
            let mut methods = HashMap::new();
            for md in &src.decl.methods {
                this.span = md.span;
                let (params, ret) = this.resolve_sig(&md.params, &md.ret_ty)?;
                if methods.insert(md.name.clone(), func_type(params, ret)).is_some() {
                    return Err(this.diag(format!("method '{}' is declared twice in component {}", md.name, src.decl.name)));
                }
            }
            let comp = &mut this.prog.comps[c as usize];
            comp.fields = fields;
            comp.methods = methods;
            Ok(())
        })
    }

    fn layout_class(&mut self, c: ir::ClassId) -> R<()> {
        let src = self.class_src[c as usize].clone();
        let decl = src.decl.clone();
        self.span = decl.span;
        let mut comps = Vec::new();
        let mut fields: Vec<ir::Field> = Vec::new();
        let mut fsrc: Vec<FieldSrc> = Vec::new();
        // method name -> (decl, module, provider name, signature)
        let mut methods: Vec<(String, Rc<ast::FnDecl>, usize, String, Type)> = Vec::new();

        for cname in &decl.components {
            let k = match self.lookup(src.module, cname) {
                Some(Symbol::Comp(k)) => k,
                Some(_) => return Err(self.diag(format!("'{cname}' is not a component"))),
                None => return Err(self.diag(format!("unknown component '{cname}'"))),
            };
            if comps.contains(&k) { return Err(self.diag(format!("component '{cname}' is listed twice"))); }
            comps.push(k);
            let comp = self.prog.comps[k as usize].clone();
            let csrc = self.comp_src[k as usize].clone();
            for ((fname, fty), fdecl) in comp.fields.iter().zip(csrc.decl.fields.iter()) {
                if let Some(prev) = fields.iter().find(|f| f.name == *fname) {
                    let other = prev.from_comp.map(|p| self.prog.comps[p as usize].name.clone()).unwrap_or_default();
                    return Err(self.diag(format!("field '{fname}' is defined by both {other} and {cname}")));
                }
                fields.push(ir::Field { name: fname.clone(), ty: fty.clone(), from_comp: Some(k) });
                fsrc.push(FieldSrc { default: fdecl.default.clone(), module: csrc.module, span: fdecl.span });
            }
            for md in &csrc.decl.methods {
                if let Some(prev) = methods.iter().find(|x| x.0 == md.name) {
                    return Err(self.diag(format!(
                        "method '{}' is provided by both {} and {cname}; define it in {} to choose", md.name, prev.3, decl.name)));
                }
                let sig = comp.methods[&md.name].clone();
                methods.push((md.name.clone(), md.clone(), csrc.module, cname.clone(), sig));
            }
        }

        let mut own_fields: Vec<String> = Vec::new();
        for f in &decl.fields {
            self.span = f.span;
            if own_fields.contains(&f.name) {
                return Err(self.diag(format!("field '{}' is declared twice in class {}", f.name, decl.name)));
            }
            own_fields.push(f.name.clone());
            let ty = self.resolve_type(&f.ty)?;
            if let Some(i) = fields.iter().position(|x| x.name == f.name) {
                if fields[i].ty != ty {
                    return Err(self.diag(format!(
                        "field '{}' overrides a component field of type {} with type {}", f.name, self.tn(&fields[i].ty), self.tn(&ty))));
                }
                if f.default.is_some() {
                    fsrc[i] = FieldSrc { default: f.default.clone(), module: src.module, span: f.span };
                }
                continue;
            }
            if f.default.is_none() && !self.has_zero(&ty) {
                return Err(self.diag(format!("field '{}' of type {} needs a default value", f.name, self.tn(&ty))));
            }
            fields.push(ir::Field { name: f.name.clone(), ty, from_comp: None });
            fsrc.push(FieldSrc { default: f.default.clone(), module: src.module, span: f.span });
        }

        for md in &decl.methods {
            self.span = md.span;
            if fields.iter().any(|f| f.name == md.name) {
                return Err(self.diag(format!("method '{}' has the same name as a field", md.name)));
            }
            let (params, ret) = self.resolve_sig(&md.params, &md.ret_ty)?;
            let sig = func_type(params, ret);
            if let Some(i) = methods.iter().position(|x| x.0 == md.name) {
                if methods[i].3 == decl.name {
                    return Err(self.diag(format!("method '{}' is declared twice in class {}", md.name, decl.name)));
                }
                if methods[i].4 != sig {
                    return Err(self.diag(format!(
                        "method '{}' overrides {}.{} with a different signature ({} vs {})",
                        md.name, methods[i].3, md.name, self.tn(&sig), self.tn(&methods[i].4))));
                }
                methods[i] = (md.name.clone(), md.clone(), src.module, decl.name.clone(), sig);
            } else {
                methods.push((md.name.clone(), md.clone(), src.module, decl.name.clone(), sig));
            }
        }

        let field_index = fields.iter().enumerate().map(|(i, f)| (f.name.clone(), i as u32)).collect();
        let mut method_map = HashMap::new();
        let mut impls = Vec::new();
        for (name, md, module, provider, sig) in methods {
            let func = self.alloc_func(&format!("{}.{}", decl.name, name), ir::FuncKind::Method { class: c }, md.span);
            self.prog.functions[func as usize].file = self.modules[module].file;
            let Type::Func { params, ret } = sig else { unreachable!() };
            let mut ps = vec![Type::Class(c)];
            ps.extend(params.iter().cloned());
            self.prog.functions[func as usize].params = ps;
            self.prog.functions[func as usize].ret = (*ret).clone();
            let _ = provider;
            method_map.insert(name, func);
            impls.push(MethodImpl { func, decl: md, module });
        }

        let init = method_map.get("init").copied();
        let mut ctor_params = Vec::new();
        if let Some(init) = init {
            let f = &self.prog.functions[init as usize];
            if f.ret != Type::Void {
                return Err(self.diag_at(f.file, f.span, format!("{}.init must not return a value", decl.name)));
            }
            ctor_params = f.params[1..].to_vec();
        }
        let default_ctor = self.alloc_func(&format!("{}.<defaults>", decl.name), ir::FuncKind::Defaults { class: c }, decl.span);
        self.prog.functions[default_ctor as usize].ret = Type::Class(c);
        let ctor = self.alloc_func(&decl.name, ir::FuncKind::Ctor { class: c }, decl.span);
        self.prog.functions[ctor as usize].params = ctor_params;
        self.prog.functions[ctor as usize].ret = Type::Class(c);

        let class = &mut self.prog.classes[c as usize];
        class.comps = comps;
        class.fields = fields;
        class.field_index = field_index;
        class.methods = method_map;
        class.ctor = ctor;
        class.default_ctor = default_ctor;
        let cs = &mut self.class_src[c as usize];
        cs.methods = impls;
        cs.fields = fsrc;
        cs.init = init;
        Ok(())
    }

    // ------------------------------------------------------------- bodies

    fn push_fn(&mut self, id: FuncId, is_lambda: bool) {
        let f = self.prog.functions[id as usize].clone();
        self.fstack.push(FnCtx { func: f, scopes: vec![HashMap::new()], local_mut: vec![], upval_mut: vec![], is_lambda, loops: 0 });
    }

    fn pop_fn(&mut self, id: FuncId, body: Vec<Stmt>) {
        let mut ctx = self.fstack.pop().expect("fn ctx");
        ctx.func.body = body;
        self.prog.functions[id as usize] = ctx.func;
    }

    fn declare_local(&mut self, name: &str, ty: Type, mutable: bool) -> R<ir::LocalId> {
        let ctx = self.fstack.last_mut().expect("in function");
        if ctx.func.locals.len() >= u16::MAX as usize {
            return Err(self.diag("too many local variables in one function".into()));
        }
        let id = ctx.func.locals.len() as ir::LocalId;
        ctx.func.locals.push(ir::Local { name: name.to_string(), ty, captured: false });
        ctx.local_mut.push(mutable);
        ctx.scopes.last_mut().unwrap().insert(name.to_string(), id);
        Ok(id)
    }

    fn check_fn_body(&mut self, id: FuncId, decl: &ast::FnDecl, self_ty: Option<Type>, m: usize) -> R<()> {
        self.with_module(m, |this| {
            this.span = decl.span;
            this.push_fn(id, false);
            let params = this.prog.functions[id as usize].params.clone();
            let mut i = 0;
            if self_ty.is_some() {
                this.declare_local("self", params[0].clone(), false)?;
                i = 1;
            }
            for p in &decl.params {
                this.declare_local(&p.name, params[i].clone(), true)?;
                i += 1;
            }
            let body = this.block_stmts(&decl.body)?;
            let ret = this.prog.functions[id as usize].ret.clone();
            if ret != Type::Void && !returns(&body) {
                this.span = decl.span;
                this.fstack.pop();
                return Err(this.diag(format!("function '{}' doesn't return a value on every path", decl.name)));
            }
            this.pop_fn(id, body);
            Ok(())
        })
    }

    fn check_comp_standalone(&mut self, c: ir::CompId) -> R<()> {
        // Check component methods once against the component type itself so
        // errors surface even if no class uses the component yet. The
        // specialised copies per class are what actually runs.
        let src = self.comp_src[c as usize].clone();
        for md in &src.decl.methods {
            let sig = self.prog.comps[c as usize].methods[&md.name].clone();
            let Type::Func { params, ret } = sig else { unreachable!() };
            let func = self.alloc_func(&format!("{}.{}", src.decl.name, md.name), ir::FuncKind::Function, md.span);
            self.prog.functions[func as usize].file = self.modules[src.module].file;
            let mut ps = vec![Type::Comp(c)];
            ps.extend(params.iter().cloned());
            self.prog.functions[func as usize].params = ps;
            self.prog.functions[func as usize].ret = (*ret).clone();
            self.check_fn_body(func, md, Some(Type::Comp(c)), src.module)?;
        }
        Ok(())
    }

    fn check_class_bodies(&mut self, c: ir::ClassId) -> R<()> {
        let src = self.class_src[c as usize].clone();
        for mi in &src.methods {
            self.check_fn_body(mi.func, &mi.decl, Some(Type::Class(c)), mi.module)?;
        }
        let class = self.prog.classes[c as usize].clone();

        // <defaults>: allocate and apply field defaults.
        self.span = src.decl.span;
        self.push_fn(class.default_ctor, false);
        let this_local = self.declare_local(" self", Type::Class(c), false)?;
        let mut body = vec![Stmt {
            kind: StmtKind::Let { local: this_local, value: Expr { span: Span::default(), kind: ExprKind::NewInstance(c), ty: Type::Class(c) } },
            span: src.decl.span,
        }];
        for (slot, (field, fs)) in class.fields.iter().zip(src.fields.iter()).enumerate() {
            let value = match &fs.default {
                Some(e) => self.with_module(fs.module, |this| {
                    this.span = fs.span;
                    this.expr_as(e, &field.ty, &format!("default of field '{}'", field.name))
                })?,
                None => Expr { span: Span::default(), kind: ExprKind::Zero, ty: field.ty.clone() },
            };
            body.push(Stmt {
                kind: StmtKind::Assign {
                    place: Place::Field { obj: Box::new(local(this_local, Type::Class(c))), slot: slot as u32 },
                    op: None,
                    value,
                },
                span: fs.span,
            });
        }
        body.push(Stmt { kind: StmtKind::Return(Some(local(this_local, Type::Class(c)))), span: src.decl.span });
        self.pop_fn(class.default_ctor, body);

        // ctor: defaults, then init(args...).
        self.push_fn(class.ctor, false);
        let params = self.prog.functions[class.ctor as usize].params.clone();
        let mut args = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let l = self.declare_local(&format!(" arg{i}"), p.clone(), false)?;
            args.push(local(l, p.clone()));
        }
        let obj = self.declare_local(" obj", Type::Class(c), false)?;
        let mut body = vec![Stmt {
            kind: StmtKind::Let {
                local: obj,
                value: Expr { span: Span::default(), kind: ExprKind::CallFunc { func: class.default_ctor, args: vec![] }, ty: Type::Class(c) },
            },
            span: src.decl.span,
        }];
        if let Some(init) = src.init {
            let mut call_args = vec![local(obj, Type::Class(c))];
            call_args.extend(args);
            body.push(Stmt {
                kind: StmtKind::Expr(Expr { span: Span::default(), kind: ExprKind::CallFunc { func: init, args: call_args }, ty: Type::Void }),
                span: src.decl.span,
            });
        }
        body.push(Stmt { kind: StmtKind::Return(Some(local(obj, Type::Class(c)))), span: src.decl.span });
        self.pop_fn(class.ctor, body);
        Ok(())
    }

    // ------------------------------------------------------------- statements

    fn block_stmts(&mut self, stmts: &[ast::Stmt]) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        for s in stmts {
            if let Some(st) = self.stmt(s)? { out.push(st); }
        }
        Ok(out)
    }

    fn scoped_block(&mut self, stmts: &[ast::Stmt]) -> R<Vec<Stmt>> {
        let level = self.fstack.len() - 1;
        self.fstack[level].scopes.push(HashMap::new());
        let r = self.block_stmts(stmts);
        self.fstack[level].scopes.pop();
        r
    }

    fn loop_body(&mut self, stmts: &[ast::Stmt]) -> R<Vec<Stmt>> {
        // Remember which function context we incremented: if checking the
        // body fails inside a nested lambda, that lambda's context may still
        // be on the stack when we get back here.
        let level = self.fstack.len() - 1;
        self.fstack[level].loops += 1;
        let r = self.scoped_block(stmts);
        self.fstack[level].loops -= 1;
        r
    }

    fn stmt(&mut self, s: &ast::Stmt) -> R<Option<Stmt>> {
        self.span = s.span;
        let span = s.span;
        let kind = match &s.kind {
            ast::StmtKind::Pass => return Ok(None),
            ast::StmtKind::Break | ast::StmtKind::Continue => {
                let is_break = matches!(s.kind, ast::StmtKind::Break);
                if self.fstack.last().unwrap().loops == 0 {
                    return Err(self.diag(format!("'{}' can only be used inside a loop", if is_break { "break" } else { "continue" })));
                }
                if is_break { StmtKind::Break } else { StmtKind::Continue }
            }
            ast::StmtKind::Binding(b) => {
                let ty = self.resolve_type(&b.ty)?;
                if ty == Type::Void { return Err(self.diag(format!("'{}' can't have type void", b.name))); }
                if matches!(b.init.kind, ast::ExprKind::Lambda { .. }) {
                    // Declared first so the lambda can call itself.
                    let l = self.declare_local(&b.name, ty.clone(), b.mutable)?;
                    let value = self.expr_as(&b.init, &ty, &format!("initializer of '{}'", b.name))?;
                    StmtKind::Let { local: l, value }
                } else {
                    let value = self.expr_as(&b.init, &ty, &format!("initializer of '{}'", b.name))?;
                    let l = self.declare_local(&b.name, ty, b.mutable)?;
                    StmtKind::Let { local: l, value }
                }
            }
            ast::StmtKind::If { cond, then_body, else_body } => {
                let cond = self.expr_as(cond, &Type::Bool, "if condition")?;
                let then_body = self.scoped_block(then_body)?;
                let else_body = match else_body { Some(b) => self.scoped_block(b)?, None => vec![] };
                StmtKind::If { cond, then_body, else_body }
            }
            ast::StmtKind::While { cond, body } => {
                let cond = self.expr_as(cond, &Type::Bool, "while condition")?;
                let body = self.loop_body(body)?;
                StmtKind::While { cond, body }
            }
            ast::StmtKind::For { var, iterable, body } => {
                let it = self.expr(iterable, None)?;
                let elem = match &it.ty {
                    Type::List { elem, .. } => (**elem).clone(),
                    Type::Map { key, .. } => (**key).clone(),
                    Type::Str => Type::Str,
                    Type::Range => Type::Int,
                    other => return Err(self.diag(format!("can't loop over a value of type {}", self.tn(other)))),
                };
                self.fstack.last_mut().unwrap().scopes.push(HashMap::new());
                let iter = self.declare_local(" iter", it.ty.clone(), false)?;
                let index = self.declare_local(" index", Type::Int, false)?;
                let v = self.declare_local(var, elem, false)?;
                let body = self.loop_body(body);
                self.fstack.last_mut().unwrap().scopes.pop();
                StmtKind::For { var: v, iter, index, iterable: it, body: body? }
            }
            ast::StmtKind::Return(value) => {
                let ret = self.fstack.last().unwrap().func.ret.clone();
                if matches!(self.fstack.last().unwrap().func.kind, ir::FuncKind::ModuleInit) {
                    return Err(self.diag("'return' outside of a function".into()));
                }
                match (value, &ret) {
                    (None, Type::Void) => StmtKind::Return(None),
                    (Some(_), Type::Void) => return Err(self.diag("this function returns void, so 'return' can't have a value".into())),
                    (None, t) => return Err(self.diag(format!("missing return value of type {}", self.tn(t)))),
                    (Some(e), t) => StmtKind::Return(Some(self.expr_as(e, t, "return value")?)),
                }
            }
            ast::StmtKind::Assign { target, op, value } => {
                let (place, ty, mutable, what) = self.place(target)?;
                if !mutable { return Err(self.diag(format!("can't assign to {what}: it is a constant"))); }
                match op {
                    ast::AssignOp::Assign => {
                        let value = self.expr_as(value, &ty, &format!("assignment to {what}"))?;
                        StmtKind::Assign { place, op: None, value }
                    }
                    ast::AssignOp::PlusAssign | ast::AssignOp::MinusAssign => {
                        let plus = *op == ast::AssignOp::PlusAssign;
                        let (bop, vty) = match (&ty, plus) {
                            (Type::Int, true) => (BinOp::AddI, Type::Int),
                            (Type::Int, false) => (BinOp::SubI, Type::Int),
                            (Type::Float, true) => (BinOp::AddF, Type::Float),
                            (Type::Float, false) => (BinOp::SubF, Type::Float),
                            (Type::Str, true) => (BinOp::Concat, Type::Str),
                            (Type::Vec(_), true) => (BinOp::VecAdd, ty.clone()),
                            (Type::Vec(_), false) => (BinOp::VecSub, ty.clone()),
                            _ => return Err(self.diag(format!(
                                "'{}' isn't supported for {what} of type {}", if plus { "+=" } else { "-=" }, self.tn(&ty)))),
                        };
                        let value = self.expr_as(value, &vty, &format!("right side of '{}'", if plus { "+=" } else { "-=" }))?;
                        StmtKind::Assign { place, op: Some(bop), value }
                    }
                }
            }
            ast::StmtKind::Expr(e) => StmtKind::Expr(self.expr(e, None)?),
        };
        Ok(Some(Stmt { kind, span }))
    }

    /// Resolve an assignment target. Returns (place, type, mutable, description).
    fn place(&mut self, target: &ast::Expr) -> R<(Place, Type, bool, String)> {
        match &target.kind {
            ast::ExprKind::Ident(name) => match self.resolve(name) {
                Some(VarRef::Local(l, ty, m)) => Ok((Place::Local(l), ty, m, format!("'{name}'"))),
                Some(VarRef::Upval(u, ty, m)) => Ok((Place::Upval(u), ty, m, format!("'{name}'"))),
                Some(VarRef::Global(g)) => {
                    let gl = &self.prog.globals[g as usize];
                    Ok((Place::Global(g), gl.ty.clone(), gl.mutable, format!("'{name}'")))
                }
                Some(_) => Err(self.diag(format!("can't assign to '{name}'"))),
                None => Err(self.diag(format!("undefined name '{name}'"))),
            },
            ast::ExprKind::Index { object, index } => {
                let obj = self.expr(object, None)?;
                match obj.ty.clone() {
                    Type::List { elem, .. } => {
                        let key = self.expr_as(index, &Type::Int, "list index")?;
                        Ok((Place::TableSlot { table: Box::new(obj), key: Box::new(key) }, (*elem).clone(), true, "table element".into()))
                    }
                    Type::Map { key: kt, val, .. } => {
                        let key = self.expr_as(index, &kt, "table key")?;
                        Ok((Place::TableSlot { table: Box::new(obj), key: Box::new(key) }, (*val).clone(), true, "table entry".into()))
                    }
                    Type::Str => Err(self.diag("strings can't be changed in place; build a new string instead".into())),
                    other => Err(self.diag(format!("values of type {} can't be indexed", self.tn(&other)))),
                }
            }
            ast::ExprKind::Field { object, field } => {
                let obj = self.expr(object, None)?;
                match &obj.ty {
                    Type::Class(c) => {
                        let class = &self.prog.classes[*c as usize];
                        match class.field_index.get(field) {
                            Some(&slot) => {
                                let ty = class.fields[slot as usize].ty.clone();
                                Ok((Place::Field { obj: Box::new(obj), slot }, ty, true, format!("field '{field}'")))
                            }
                            None => Err(self.diag(format!("{} has no field '{field}'", class.name))),
                        }
                    }
                    Type::Comp(k) => {
                        let comp = &self.prog.comps[*k as usize];
                        match comp.fields.iter().find(|(n, _)| n == field) {
                            Some((_, ty)) => {
                                let ty = ty.clone();
                                Ok((Place::FieldNamed { obj: Box::new(obj), name: field.as_str().into() }, ty, true, format!("field '{field}'")))
                            }
                            None => Err(self.diag(format!("component {} has no field '{field}'", comp.name))),
                        }
                    }
                    Type::Vec(n) => {
                        let index = vec_index(field, *n).ok_or_else(|| self.diag(format!("vec{n} has no component '{field}'")))?;
                        let (base, _, mutable, what) = self.place(object)?;
                        Ok((Place::VecComp { base: Box::new(base), index }, Type::Float, mutable, format!("{what}.{field}")))
                    }
                    other => Err(self.diag(format!("values of type {} have no fields", self.tn(other)))),
                }
            }
            _ => Err(self.diag("this expression can't be assigned to".into())),
        }
    }

    // ------------------------------------------------------------- names

    fn lookup(&self, m: usize, name: &str) -> Option<Symbol> { self.modules[m].scope.get(name).cloned() }

    fn resolve(&mut self, name: &str) -> Option<VarRef> {
        if !self.fstack.is_empty() {
            if let Some(v) = self.resolve_local(self.fstack.len() - 1, name) { return Some(v); }
        }
        match self.lookup(self.module, name) {
            Some(Symbol::Global(g)) => Some(VarRef::Global(g)),
            Some(sym) => Some(VarRef::Sym(sym)),
            None => builtins::PRELUDE.iter().find(|p| **p == name).map(|p| VarRef::Prelude(p)),
        }
    }

    fn resolve_local(&mut self, level: usize, name: &str) -> Option<VarRef> {
        {
            let ctx = &self.fstack[level];
            for scope in ctx.scopes.iter().rev() {
                if let Some(&l) = scope.get(name) {
                    return Some(VarRef::Local(l, ctx.func.locals[l as usize].ty.clone(), ctx.local_mut[l as usize]));
                }
            }
            if let Some(u) = ctx.func.upvalues.iter().position(|u| u.name == name) {
                return Some(VarRef::Upval(u as u16, ctx.func.upvalues[u].ty.clone(), ctx.upval_mut[u]));
            }
            if !ctx.is_lambda || level == 0 { return None; }
        }
        let (source, ty, mutable) = match self.resolve_local(level - 1, name)? {
            VarRef::Local(l, ty, m) => {
                self.fstack[level - 1].func.locals[l as usize].captured = true;
                (UpvalSource::Local(l), ty, m)
            }
            VarRef::Upval(u, ty, m) => (UpvalSource::Upval(u), ty, m),
            _ => return None,
        };
        let ctx = &mut self.fstack[level];
        ctx.func.upvalues.push(ir::Upvalue { name: name.to_string(), ty: ty.clone(), source });
        ctx.upval_mut.push(mutable);
        Some(VarRef::Upval((ctx.func.upvalues.len() - 1) as u16, ty, mutable))
    }

    fn resolve_type(&mut self, t: &ast::TypeExpr) -> R<Type> {
        use ast::TypeExpr as T;
        Ok(match t {
            T::Int => Type::Int,
            T::Float => Type::Float,
            T::Str => Type::Str,
            T::Bool => Type::Bool,
            T::Void => Type::Void,
            T::Vec2 => Type::Vec(2),
            T::Vec3 => Type::Vec(3),
            T::Vec4 => Type::Vec(4),
            T::Table1D(_) | T::Table2D(..) => return Err(self.diag("table types need a fixed capacity, e.g. T32[i]".into())),
            T::FixedTable1D { capacity, element } => {
                let elem = self.resolve_type(element)?;
                if elem == Type::Void { return Err(self.diag("table elements can't be void".into())); }
                Type::List { cap: *capacity, elem: Rc::new(elem) }
            }
            T::FixedTable2D { capacity, key, value } => {
                let k = self.resolve_type(key)?;
                if !matches!(k, Type::Int | Type::Str | Type::Bool) {
                    return Err(self.diag(format!("table keys must be i, s or bool, not {}", self.tn(&k))));
                }
                let v = self.resolve_type(value)?;
                if v == Type::Void { return Err(self.diag("table values can't be void".into())); }
                Type::Map { cap: *capacity, key: Rc::new(k), val: Rc::new(v) }
            }
            T::Function { params, ret } => {
                let mut ps = Vec::new();
                for p in params { ps.push(self.resolve_type(p)?); }
                func_type(ps, self.resolve_type(ret)?)
            }
            T::Named(name) => match self.lookup(self.module, name) {
                Some(Symbol::Class(c)) => Type::Class(c),
                Some(Symbol::Comp(k)) => Type::Comp(k),
                Some(_) => return Err(self.diag(format!("'{name}' is not a type"))),
                None => return Err(self.diag(format!("unknown type '{name}'"))),
            },
        })
    }

    fn has_zero(&self, t: &Type) -> bool {
        !matches!(t, Type::Func { .. } | Type::Comp(_) | Type::Signal(_) | Type::Range | Type::Any | Type::Void)
    }

    // ------------------------------------------------------------- expressions

    fn expr_as(&mut self, e: &ast::Expr, target: &Type, what: &str) -> R<Expr> {
        let ex = self.expr(e, Some(target))?;
        let saved = self.span;
        self.span = e.span;
        let r = self.coerce(ex, target, what);
        self.span = saved;
        r
    }

    fn coerce(&self, ex: Expr, target: &Type, what: &str) -> R<Expr> {
        if ex.ty == *target { return Ok(ex); }
        match (&ex.ty, target) {
            (Type::Int, Type::Float) => Ok(Expr { span: Span::default(), kind: ExprKind::IntToFloat(Box::new(ex)), ty: Type::Float }),
            (Type::Void, _) => Err(self.diag(format!("{what}: this expression has no value (void)"))),
            (_, Type::Any) => Ok(ex),
            (Type::Class(c), Type::Comp(k)) if self.prog.classes[*c as usize].comps.contains(k) => Ok(Expr { ty: target.clone(), ..ex }),
            _ => Err(self.diag(format!("{what}: expected {}, found {}", self.tn(target), self.tn(&ex.ty)))),
        }
    }

    /// Check an expression. Errors inside it point at its own source span.
    fn expr(&mut self, e: &ast::Expr, expected: Option<&Type>) -> R<Expr> {
        let saved = self.span;
        self.span = e.span;
        let r = self.expr_inner(e, expected);
        self.span = saved;
        let mut ex = r?;
        ex.span = e.span;
        Ok(ex)
    }

    fn expr_inner(&mut self, e: &ast::Expr, expected: Option<&Type>) -> R<Expr> {
        use ast::ExprKind as A;
        Ok(match &e.kind {
            A::IntLit(n) => Expr { span: Span::default(), kind: ExprKind::Int(*n), ty: Type::Int },
            A::FloatLit(x) => Expr { span: Span::default(), kind: ExprKind::Float(*x), ty: Type::Float },
            A::StrLit(s) => Expr { span: Span::default(), kind: ExprKind::Str(s.as_str().into()), ty: Type::Str },
            A::BoolLit(b) => Expr { span: Span::default(), kind: ExprKind::Bool(*b), ty: Type::Bool },
            A::Ident(name) => self.ident(name)?,
            A::TableLit(entries) => self.table_lit(entries, expected)?,
            A::Unary { op, expr } => {
                let inner = self.expr(expr, expected)?;
                match (op, &inner.ty) {
                    (ast::UnaryOp::Neg, Type::Int) => un(UnOp::NegI, inner, Type::Int),
                    (ast::UnaryOp::Neg, Type::Float) => un(UnOp::NegF, inner, Type::Float),
                    (ast::UnaryOp::Neg, Type::Vec(n)) => { let n = *n; un(UnOp::NegV, inner, Type::Vec(n)) }
                    (ast::UnaryOp::Not, Type::Bool) => un(UnOp::Not, inner, Type::Bool),
                    (ast::UnaryOp::Neg, t) => return Err(self.diag(format!("can't negate a value of type {}", self.tn(t)))),
                    (ast::UnaryOp::Not, t) => return Err(self.diag(format!("'not' needs a bool, found {}", self.tn(t)))),
                }
            }
            A::Binary { left, op, right } => self.binary(left, *op, right)?,
            A::Field { object, field } => self.field(object, field)?,
            A::Call { callee, args } => self.call(callee, args, expected)?,
            A::MethodCall { object, method, args } => self.method_call(object, method, args, expected)?,
            A::Index { object, index } => self.index(object, index)?,
            A::Lambda { params, ret_ty, body } => self.lambda(params, ret_ty, body)?,
        })
    }

    /// Evaluate a constant initializer at compile time: literals, other
    /// known constants, math constants, and arithmetic on them. `None` for
    /// anything else (or anything that would fail, like dividing by zero).
    fn fold_const(&mut self, e: &ast::Expr) -> Option<ConstVal> {
        use ast::ExprKind as A;
        use ConstVal::*;
        Some(match &e.kind {
            A::IntLit(n) => I(*n),
            A::FloatLit(x) => F(*x),
            A::StrLit(t) => S(t.clone()),
            A::BoolLit(b) => B(*b),
            A::Ident(name) => match self.resolve(name)? {
                VarRef::Global(g) => match self.consts.get(&g)? {
                    ExprKind::Int(n) => I(*n),
                    ExprKind::Float(x) => F(*x),
                    ExprKind::Str(t) => S(t.to_string()),
                    ExprKind::Bool(b) => B(*b),
                    _ => return None,
                },
                VarRef::Sym(Symbol::MathConst(c)) => F(c),
                _ => return None,
            },
            A::Unary { op: ast::UnaryOp::Neg, expr } => match self.fold_const(expr)? {
                I(n) => I(n.checked_neg()?),
                F(x) => F(-x),
                _ => return None,
            },
            A::Binary { left, op, right } => {
                let (l, r) = (self.fold_const(left)?, self.fold_const(right)?);
                use ast::BinaryOp as O;
                match (l, r) {
                    (I(a), I(b)) => I(match op {
                        O::Add => a.checked_add(b)?, O::Sub => a.checked_sub(b)?, O::Mul => a.checked_mul(b)?,
                        O::Div => a.checked_div(b)?, O::Mod => a.checked_rem_euclid(b)?,
                        _ => return None,
                    }),
                    (S(a), S(b)) if *op == O::Add => S(a + &b),
                    (a, b) => {
                        let f = |v: ConstVal| match v { I(n) => Some(n as f64), F(x) => Some(x), _ => None };
                        let (a, b) = (f(a)?, f(b)?);
                        F(match op { O::Add => a + b, O::Sub => a - b, O::Mul => a * b, O::Div => a / b, _ => return None })
                    }
                }
            }
            _ => return None,
        })
    }

    fn ident(&mut self, name: &str) -> R<Expr> {
        match self.resolve(name) {
            Some(VarRef::Local(l, ty, _)) => Ok(Expr { span: Span::default(), kind: ExprKind::Local(l), ty }),
            Some(VarRef::Upval(u, ty, _)) => Ok(Expr { span: Span::default(), kind: ExprKind::Upval(u), ty }),
            Some(VarRef::Global(g)) => {
                let ty = self.prog.globals[g as usize].ty.clone();
                if let Some(k) = self.consts.get(&g) { return Ok(Expr { span: Span::default(), kind: k.clone(), ty }); }
                Ok(Expr { span: Span::default(), kind: ExprKind::Global(g), ty })
            }
            Some(VarRef::Sym(Symbol::Func(f))) => {
                let func = &self.prog.functions[f as usize];
                Ok(Expr { span: Span::default(), kind: ExprKind::Closure { func: f, captures: vec![] }, ty: func_type(func.params.clone(), func.ret.clone()) })
            }
            Some(VarRef::Sym(Symbol::Signal(s))) => {
                let g = self.prog.signals[s as usize].global;
                Ok(Expr { span: Span::default(), kind: ExprKind::Global(g), ty: Type::Signal(s) })
            }
            Some(VarRef::Sym(Symbol::MathConst(c))) => Ok(Expr { span: Span::default(), kind: ExprKind::Float(c), ty: Type::Float }),
            Some(VarRef::Sym(Symbol::Class(_))) => Err(self.diag(format!("class '{name}' isn't a value; call {name}() to create an instance"))),
            Some(VarRef::Sym(Symbol::Comp(_))) => Err(self.diag(format!("component '{name}' isn't a value"))),
            Some(VarRef::Sym(Symbol::MathFn(_))) | Some(VarRef::Prelude(_)) | Some(VarRef::Sym(Symbol::Host(_))) =>
                Err(self.diag(format!("built-in function '{name}' can only be called, not used as a value"))),
            Some(VarRef::Sym(Symbol::Global(_))) => unreachable!(),
            None => Err(self.diag(format!("undefined name '{name}'"))),
        }
    }

    fn table_lit(&mut self, entries: &[ast::TableEntry], expected: Option<&Type>) -> R<Expr> {
        let Some(ty) = expected else {
            return Err(self.diag("a table literal needs a known type here; bind it to a typed variable first, e.g. var t: T8[i] = {1, 2}".into()));
        };
        match ty {
            Type::List { cap, elem } => {
                if entries.len() > *cap {
                    return Err(self.diag(format!("table literal has {} items but {} holds at most {cap}", entries.len(), self.tn(ty))));
                }
                let mut items = Vec::new();
                for (i, en) in entries.iter().enumerate() {
                    match en {
                        ast::TableEntry::Item(e) => items.push(self.expr_as(e, elem, &format!("table item {}", i + 1))?),
                        ast::TableEntry::Pair(..) => return Err(self.diag(format!("{} is a list table; use plain items, not key: value", self.tn(ty)))),
                    }
                }
                Ok(Expr { span: Span::default(), kind: ExprKind::MakeList(items), ty: ty.clone() })
            }
            Type::Map { cap, key, val } => {
                if entries.len() > *cap {
                    return Err(self.diag(format!("table literal has {} entries but {} holds at most {cap}", entries.len(), self.tn(ty))));
                }
                let mut pairs = Vec::new();
                for (i, en) in entries.iter().enumerate() {
                    match en {
                        ast::TableEntry::Pair(k, v) => {
                            let k = self.expr_as(k, key, &format!("key {}", i + 1))?;
                            let v = self.expr_as(v, val, &format!("value {}", i + 1))?;
                            pairs.push((k, v));
                        }
                        ast::TableEntry::Item(_) => return Err(self.diag(format!("{} is a key/value table; write entries as key: value", self.tn(ty)))),
                    }
                }
                Ok(Expr { span: Span::default(), kind: ExprKind::MakeMap(pairs), ty: ty.clone() })
            }
            Type::Vec(n) => {
                if entries.len() != *n as usize {
                    return Err(self.diag(format!("vec{n} needs exactly {n} components, found {}", entries.len())));
                }
                let mut comps = Vec::new();
                for (i, en) in entries.iter().enumerate() {
                    match en {
                        ast::TableEntry::Item(e) => comps.push(self.expr_as(e, &Type::Float, &format!("vec{n} component {}", i + 1))?),
                        ast::TableEntry::Pair(..) => return Err(self.diag("vector literals take plain numbers".into())),
                    }
                }
                Ok(Expr { span: Span::default(), kind: ExprKind::MakeVec(comps), ty: ty.clone() })
            }
            other => Err(self.diag(format!("a table literal can't be used where {} is expected", self.tn(other)))),
        }
    }

    fn binary(&mut self, left: &ast::Expr, op: ast::BinaryOp, right: &ast::Expr) -> R<Expr> {
        use ast::BinaryOp as B;
        if matches!(op, B::And | B::Or) {
            let l = self.expr_as(left, &Type::Bool, if op == B::And { "left side of 'and'" } else { "left side of 'or'" })?;
            let r = self.expr_as(right, &Type::Bool, if op == B::And { "right side of 'and'" } else { "right side of 'or'" })?;
            let kind = if op == B::And { ExprKind::And(Box::new(l), Box::new(r)) } else { ExprKind::Or(Box::new(l), Box::new(r)) };
            return Ok(Expr { span: Span::default(), kind, ty: Type::Bool });
        }
        // A table literal operand takes its type from the other side (e.g. v + {1.0, 0.0}).
        let (l, r) = if matches!(left.kind, ast::ExprKind::TableLit(_)) && !matches!(right.kind, ast::ExprKind::TableLit(_)) {
            let r = self.expr(right, None)?;
            let l = self.expr(left, Some(&r.ty))?;
            (l, r)
        } else {
            let l = self.expr(left, None)?;
            let r = self.expr(right, Some(&l.ty))?;
            (l, r)
        };
        let sym = match op { B::Add => "+", B::Sub => "-", B::Mul => "*", B::Div => "/", B::Mod => "%", B::Eq => "==", B::NotEq => "!=", B::Lt => "<", B::LtEq => "<=", B::Gt => ">", B::GtEq => ">=", _ => "?" };
        let mismatch = |this: &Self, l: &Expr, r: &Expr| -> Diagnostic {
            this.diag(format!("'{sym}' can't be applied to {} and {}", this.tn(&l.ty), this.tn(&r.ty)))
        };
        let float = |e: Expr| if e.ty == Type::Int { Expr { span: Span::default(), kind: ExprKind::IntToFloat(Box::new(e)), ty: Type::Float } } else { e };
        let bin = |op: BinOp, l: Expr, r: Expr, ty: Type| Expr { span: Span::default(), kind: ExprKind::Binary(op, Box::new(l), Box::new(r)), ty };

        match op {
            B::Add | B::Sub | B::Mul | B::Div | B::Mod => {
                let (iop, fop) = match op {
                    B::Add => (BinOp::AddI, BinOp::AddF), B::Sub => (BinOp::SubI, BinOp::SubF),
                    B::Mul => (BinOp::MulI, BinOp::MulF), B::Div => (BinOp::DivI, BinOp::DivF),
                    _ => (BinOp::ModI, BinOp::ModF),
                };
                match (&l.ty, &r.ty) {
                    (Type::Int, Type::Int) => Ok(bin(iop, l, r, Type::Int)),
                    (a, b) if a.is_numeric() && b.is_numeric() => Ok(bin(fop, float(l), float(r), Type::Float)),
                    (Type::Str, Type::Str) if op == B::Add => Ok(bin(BinOp::Concat, l, r, Type::Str)),
                    (Type::Vec(a), Type::Vec(b)) if a == b && matches!(op, B::Add | B::Sub | B::Mul) => {
                        let vop = match op { B::Add => BinOp::VecAdd, B::Sub => BinOp::VecSub, _ => BinOp::VecMul };
                        let t = l.ty.clone();
                        Ok(bin(vop, l, r, t))
                    }
                    (Type::Vec(_), b) if b.is_numeric() && matches!(op, B::Mul | B::Div) => {
                        let t = l.ty.clone();
                        Ok(bin(if op == B::Mul { BinOp::VecScale } else { BinOp::VecDiv }, l, float(r), t))
                    }
                    (a, Type::Vec(_)) if a.is_numeric() && op == B::Mul => {
                        let t = r.ty.clone();
                        Ok(bin(BinOp::ScaleVec, float(l), r, t))
                    }
                    _ => Err(mismatch(self, &l, &r)),
                }
            }
            B::Lt | B::LtEq | B::Gt | B::GtEq => {
                let c = match op { B::Lt => Cmp::Lt, B::LtEq => Cmp::Le, B::Gt => Cmp::Gt, _ => Cmp::Ge };
                match (&l.ty, &r.ty) {
                    (Type::Int, Type::Int) => Ok(bin(BinOp::CmpI(c), l, r, Type::Bool)),
                    (a, b) if a.is_numeric() && b.is_numeric() => Ok(bin(BinOp::CmpF(c), float(l), float(r), Type::Bool)),
                    (Type::Str, Type::Str) => Ok(bin(BinOp::CmpS(c), l, r, Type::Bool)),
                    _ => Err(mismatch(self, &l, &r)),
                }
            }
            B::Eq | B::NotEq => {
                let o = if op == B::Eq { BinOp::Eq } else { BinOp::Ne };
                if l.ty.is_numeric() && r.ty.is_numeric() && l.ty != r.ty {
                    return Ok(bin(o, float(l), float(r), Type::Bool));
                }
                let compatible = l.ty == r.ty
                    || matches!((&l.ty, &r.ty), (Type::Class(_), Type::Comp(_)) | (Type::Comp(_), Type::Class(_)) | (Type::Comp(_), Type::Comp(_)));
                if !compatible || l.ty == Type::Void { return Err(mismatch(self, &l, &r)); }
                Ok(bin(o, l, r, Type::Bool))
            }
            B::And | B::Or => unreachable!(),
        }
    }

    fn index(&mut self, object: &ast::Expr, index: &ast::Expr) -> R<Expr> {
        let obj = self.expr(object, None)?;
        match obj.ty.clone() {
            Type::List { elem, .. } => {
                let key = self.expr_as(index, &Type::Int, "list index")?;
                Ok(Expr::new(ExprKind::Table { op: TableOp::Get, table: Box::new(obj), args: vec![key] }, (*elem).clone()))
            }
            Type::Map { key: kt, val, .. } => {
                let key = self.expr_as(index, &kt, "table key")?;
                Ok(Expr::new(ExprKind::Table { op: TableOp::Get, table: Box::new(obj), args: vec![key] }, (*val).clone()))
            }
            Type::Str => {
                let key = self.expr_as(index, &Type::Int, "string index")?;
                Ok(Expr::new(ExprKind::CallBuiltin { builtin: ir::Builtin::StrAt, args: vec![obj, key] }, Type::Str))
            }
            other => Err(self.diag(format!("values of type {} can't be indexed", self.tn(&other)))),
        }
    }

    fn field(&mut self, object: &ast::Expr, field: &str) -> R<Expr> {
        let obj = self.expr(object, None)?;
        match obj.ty.clone() {
            Type::Class(c) => {
                let class = &self.prog.classes[c as usize];
                if let Some(&slot) = class.field_index.get(field) {
                    let ty = class.fields[slot as usize].ty.clone();
                    return Ok(Expr { span: Span::default(), kind: ExprKind::GetField { obj: Box::new(obj), slot }, ty });
                }
                if let Some(&func) = class.methods.get(field) {
                    let f = &self.prog.functions[func as usize];
                    let ty = func_type(f.params[1..].to_vec(), f.ret.clone());
                    return Ok(Expr { span: Span::default(), kind: ExprKind::BindMethod { recv: Box::new(obj), func }, ty });
                }
                Err(self.diag(format!("{} has no field or method '{field}'", class.name)))
            }
            Type::Comp(k) => {
                let comp = &self.prog.comps[k as usize];
                if let Some((_, ty)) = comp.fields.iter().find(|(n, _)| n == field) {
                    let ty = ty.clone();
                    return Ok(Expr { span: Span::default(), kind: ExprKind::GetFieldNamed { obj: Box::new(obj), name: field.into() }, ty });
                }
                if let Some(sig) = comp.methods.get(field) {
                    let ty = sig.clone();
                    return Ok(Expr { span: Span::default(), kind: ExprKind::BindMethodNamed { recv: Box::new(obj), name: field.into() }, ty });
                }
                Err(self.diag(format!("component {} has no field or method '{field}'", comp.name)))
            }
            Type::Vec(n) => {
                let index = vec_index(field, n).ok_or_else(|| self.diag(format!("vec{n} has no component '{field}'")))?;
                Ok(Expr { span: Span::default(), kind: ExprKind::VecComp { vec: Box::new(obj), index }, ty: Type::Float })
            }
            Type::List { .. } | Type::Map { .. } => Err(self.diag(format!(
                "tables have no fields; use .iter(...), .pos(...), .edit(...) or .view(...) (e.g. t.iter(len))"))),
            other => Err(self.diag(format!("values of type {} have no field '{field}'", self.tn(&other)))),
        }
    }

    fn args_for(&mut self, what: &str, params: &[Type], args: &[ast::Expr]) -> R<Vec<Expr>> {
        if params.len() != args.len() {
            return Err(self.diag(format!("{what} expects {} argument{}, got {}", params.len(), if params.len() == 1 { "" } else { "s" }, args.len())));
        }
        let mut out = Vec::new();
        for (i, (p, a)) in params.iter().zip(args).enumerate() {
            out.push(self.expr_as(a, p, &format!("argument {} of {what}", i + 1))?);
        }
        Ok(out)
    }

    fn call(&mut self, callee: &ast::Expr, args: &[ast::Expr], expected: Option<&Type>) -> R<Expr> {
        if let ast::ExprKind::Ident(name) = &callee.kind {
            match self.resolve(name) {
                Some(VarRef::Sym(Symbol::Func(f))) => {
                    let func = self.prog.functions[f as usize].clone();
                    let args = self.args_for(&format!("{name}()"), &func.params, args)?;
                    return Ok(Expr { span: Span::default(), kind: ExprKind::CallFunc { func: f, args }, ty: func.ret });
                }
                Some(VarRef::Sym(Symbol::Class(c))) => {
                    let ctor = self.prog.classes[c as usize].ctor;
                    let params = self.prog.functions[ctor as usize].params.clone();
                    let args = self.args_for(&format!("{name}()"), &params, args)?;
                    return Ok(Expr { span: Span::default(), kind: ExprKind::CallFunc { func: ctor, args }, ty: Type::Class(c) });
                }
                Some(VarRef::Sym(Symbol::Comp(_))) =>
                    return Err(self.diag(format!("component '{name}' can't be created on its own; include it in a class, e.g. class Thing:({name}) {{ }}"))),
                Some(VarRef::Sym(Symbol::MathFn(f))) => return self.builtin_call(f, args, expected),
                Some(VarRef::Sym(Symbol::Host(id))) => {
                    let sig = self.host_fns[id as usize].clone();
                    let args = self.args_for(&format!("{}()", sig.name), &sig.params, args)?;
                    return Ok(Expr::new(ExprKind::CallBuiltin { builtin: ir::Builtin::Host(id), args }, sig.ret));
                }
                Some(VarRef::Prelude(p)) => return self.builtin_call(p, args, expected),
                Some(VarRef::Sym(Symbol::Signal(s))) => {
                    let g = self.prog.signals[s as usize].global;
                    let sig = Expr { span: Span::default(), kind: ExprKind::Global(g), ty: Type::Signal(s) };
                    return self.signal_emit(sig, s, args);
                }
                _ => {}
            }
        }
        let callee = self.expr(callee, None)?;
        match callee.ty.clone() {
            Type::Func { params, ret } => {
                let args = self.args_for("this function", &params, args)?;
                Ok(Expr { span: Span::default(), kind: ExprKind::CallValue { callee: Box::new(callee), args }, ty: (*ret).clone() })
            }
            Type::Signal(s) => self.signal_emit(callee, s, args),
            other => Err(self.diag(format!("a value of type {} can't be called", self.tn(&other)))),
        }
    }

    fn builtin_call(&mut self, name: &str, args: &[ast::Expr], expected: Option<&Type>) -> R<Expr> {
        let mut checked = Vec::new();
        for a in args { checked.push(self.expr(a, None)?); }
        if builtins::needs_capacity(name) {
            if checked.len() != 1 { return Err(self.diag(format!("{name}() expects 1 argument, got {}", checked.len()))); }
            let cap = match expected {
                Some(Type::List { cap, elem }) if **elem == Type::Str => *cap,
                _ => return Err(self.diag(format!(
                    "{name} needs to know how many entries to allow; bind it to a typed table, e.g. var lines: T256[s] = {name}(path)"))),
            };
            let path = self.coerce(checked.pop().unwrap(), &Type::Str, &format!("argument 1 of {name}()"))?;
            let b = if name == "read_lines" { ir::Builtin::ReadLines } else { ir::Builtin::ListDir };
            let cap_arg = Expr::new(ExprKind::Int(cap as i64), Type::Int);
            return Ok(Expr::new(ExprKind::CallBuiltin { builtin: b, args: vec![path, cap_arg] }, Type::list(cap, Type::Str)));
        }
        let types: Vec<Type> = checked.iter().map(|e| e.ty.clone()).collect();
        let resolved = builtins::resolve_function(name, &types).map_err(|m| self.diag(m))?;
        self.finish_builtin(name, resolved, checked)
    }

    fn finish_builtin(&mut self, name: &str, resolved: Resolved, checked: Vec<Expr>) -> R<Expr> {
        match resolved {
            Resolved::Identity => Ok(checked.into_iter().next().unwrap()),
            Resolved::Variadic { builtin, ret } => {
                let mut out = Vec::new();
                for (i, a) in checked.into_iter().enumerate() { out.push(self.coerce(a, &Type::Any, &format!("argument {} of {name}()", i + 1))?); }
                Ok(Expr { span: Span::default(), kind: ExprKind::CallBuiltin { builtin, args: out }, ty: ret })
            }
            Resolved::Call { builtin, params, ret } => {
                let mut out = Vec::new();
                for (i, (a, p)) in checked.into_iter().zip(params.iter()).enumerate() {
                    out.push(self.coerce(a, p, &format!("argument {} of {name}()", i + 1))?);
                }
                Ok(Expr { span: Span::default(), kind: ExprKind::CallBuiltin { builtin, args: out }, ty: ret })
            }
        }
    }

    fn signal_emit(&mut self, sig: Expr, s: ir::SignalId, args: &[ast::Expr]) -> R<Expr> {
        let info = self.prog.signals[s as usize].clone();
        if args.len() != info.params.len() {
            return Err(self.diag(format!("signal '{}' takes {} argument{} ({}), got {}",
                info.name, info.params.len(), if info.params.len() == 1 { "" } else { "s" }, info.params.join(", "), args.len())));
        }
        let mut out = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let e = self.expr(a, None)?;
            out.push(self.coerce(e, &Type::Any, &format!("argument '{}' of signal '{}'", info.params[i], info.name))?);
        }
        Ok(Expr { span: Span::default(), kind: ExprKind::SignalEmit { signal: Box::new(sig), args: out }, ty: Type::Void })
    }

    fn method_call(&mut self, object: &ast::Expr, method: &str, args: &[ast::Expr], expected: Option<&Type>) -> R<Expr> {
        let obj = self.expr(object, None)?;
        match obj.ty.clone() {
            Type::Class(c) => {
                let class = self.prog.classes[c as usize].clone();
                if let Some(&func) = class.methods.get(method) {
                    let f = self.prog.functions[func as usize].clone();
                    let mut all = vec![obj];
                    all.extend(self.args_for(&format!("{}.{method}()", class.name), &f.params[1..], args)?);
                    return Ok(Expr { span: Span::default(), kind: ExprKind::CallFunc { func, args: all }, ty: f.ret });
                }
                if let Some(&slot) = class.field_index.get(method) {
                    if let Type::Func { params, ret } = class.fields[slot as usize].ty.clone() {
                        let callee = Expr { span: Span::default(), kind: ExprKind::GetField { obj: Box::new(obj), slot }, ty: class.fields[slot as usize].ty.clone() };
                        let args = self.args_for(&format!("{}.{method}", class.name), &params, args)?;
                        return Ok(Expr { span: Span::default(), kind: ExprKind::CallValue { callee: Box::new(callee), args }, ty: (*ret).clone() });
                    }
                }
                Err(self.diag(format!("{} has no method '{method}'", class.name)))
            }
            Type::Comp(k) => {
                let comp = self.prog.comps[k as usize].clone();
                if let Some(Type::Func { params, ret }) = comp.methods.get(method).cloned() {
                    let args = self.args_for(&format!("{}.{method}()", comp.name), &params, args)?;
                    return Ok(Expr { span: Span::default(), kind: ExprKind::CallMethodNamed { recv: Box::new(obj), name: method.into(), args }, ty: (*ret).clone() });
                }
                if let Some((_, fty @ Type::Func { .. })) = comp.fields.iter().find(|(n, _)| n == method).cloned() {
                    let Type::Func { params, ret } = fty.clone() else { unreachable!() };
                    let callee = Expr { span: Span::default(), kind: ExprKind::GetFieldNamed { obj: Box::new(obj), name: method.into() }, ty: fty };
                    let args = self.args_for(&format!("{}.{method}", comp.name), &params, args)?;
                    return Ok(Expr { span: Span::default(), kind: ExprKind::CallValue { callee: Box::new(callee), args }, ty: (*ret).clone() });
                }
                Err(self.diag(format!("component {} has no method '{method}'", comp.name)))
            }
            Type::List { .. } | Type::Map { .. } => self.table_call(obj, method, args),
            Type::Signal(s) => match method {
                "listen" => {
                    if args.len() != 1 { return Err(self.diag(".listen() takes one handler function".into())); }
                    let h = self.expr(&args[0], None)?;
                    let n = self.prog.signals[s as usize].params.len();
                    match &h.ty {
                        Type::Func { params, .. } if params.len() == n => {}
                        Type::Func { params, .. } => return Err(self.diag(format!(
                            "signal '{}' passes {n} argument{} but this handler takes {}",
                            self.prog.signals[s as usize].name, if n == 1 { "" } else { "s" }, params.len()))),
                        other => return Err(self.diag(format!(".listen() needs a function, found {}", self.tn(other)))),
                    }
                    Ok(Expr { span: Span::default(), kind: ExprKind::SignalListen { signal: Box::new(obj), handler: Box::new(h) }, ty: Type::Void })
                }
                "emit" => self.signal_emit(obj, s, args),
                _ => Err(self.diag(format!("signals have .listen(handler) and .emit(...), not .{method}()"))),
            },
            Type::Str if method == "split" => {
                if args.len() != 1 { return Err(self.diag(format!(".split() expects 1 argument, got {}", args.len()))); }
                let sep = self.expr_as(&args[0], &Type::Str, "argument 1 of .split()")?;
                let cap = match expected {
                    Some(Type::List { cap, elem }) if **elem == Type::Str => *cap,
                    _ => return Err(self.diag(
                        "split needs to know how many pieces to allow; bind it to a typed table, e.g. var parts: T16[s] = line.split(\",\")".into())),
                };
                let cap_arg = Expr::new(ExprKind::Int(cap as i64), Type::Int);
                Ok(Expr::new(ExprKind::CallBuiltin { builtin: ir::Builtin::StrSplit, args: vec![obj, sep, cap_arg] }, Type::list(cap, Type::Str)))
            }
            ty @ (Type::Str | Type::Vec(_)) => {
                let mut checked = Vec::new();
                for a in args { checked.push(self.expr(a, None)?); }
                let types: Vec<Type> = checked.iter().map(|e| e.ty.clone()).collect();
                match builtins::resolve_method(&ty, method, &types) {
                    Some(r) => {
                        let r = r.map_err(|m| self.diag(m))?;
                        let mut all = vec![obj];
                        all.extend(checked);
                        self.finish_builtin(method, r, all)
                    }
                    None => Err(self.diag(format!("{} has no method '{method}'", self.tn(&ty)))),
                }
            }
            other => Err(self.diag(format!("values of type {} have no method '{method}'", self.tn(&other)))),
        }
    }

    fn table_call(&mut self, table: Expr, dispatcher: &str, args: &[ast::Expr]) -> R<Expr> {
        let ops = TableOp::ops_for(dispatcher);
        if ops.is_empty() {
            return Err(self.diag(format!("tables have no method '{dispatcher}'; use .iter(...), .pos(...), .edit(...) or .view(...)")));
        }
        let expected = ops.join(", ");
        let Some(first) = args.first() else {
            return Err(self.diag(format!(".{dispatcher}(...) needs an operation as its first argument: one of {expected}")));
        };
        let ast::ExprKind::Ident(opname) = &first.kind else {
            return Err(self.diag(format!("the first argument of .{dispatcher}(...) must be an operation name: one of {expected}")));
        };
        let Some(op) = TableOp::lookup(dispatcher, opname) else {
            let hint = ["iter", "pos", "edit", "view"].into_iter()
                .find(|d| TableOp::lookup(d, opname).is_some())
                .map(|d| format!(" ('{opname}' belongs to .{d}(...))"))
                .unwrap_or_default();
            return Err(self.diag(format!("unknown operation '{opname}' for .{dispatcher}(...); expected one of {expected}{hint}")));
        };
        let rest = &args[1..];
        let what = format!(".{dispatcher}({opname}, ...)");
        let tty = table.ty.clone();
        let (is_list, cap, key, val) = match &tty {
            Type::List { cap, elem } => (true, *cap, Type::Int, (**elem).clone()),
            Type::Map { cap, key, val } => (false, *cap, (**key).clone(), (**val).clone()),
            _ => unreachable!(),
        };
        let list_only = |this: &Self, alt: &str| -> Diagnostic {
            this.diag(format!("{what} only works on list tables; {} is a key/value table{alt}", this.tn(&tty)))
        };
        let (params, ret): (Vec<Type>, Type) = match op {
            TableOp::Len => (vec![], Type::Int),
            TableOp::Sum => match &val {
                Type::Int | Type::Float => (vec![], val.clone()),
                _ => return Err(self.diag(format!("{what} needs a table of i or f values, found {}", self.tn(&tty)))),
            },
            TableOp::Even => {
                if !is_list || val != Type::Int {
                    return Err(self.diag(format!("{what} needs a T[i] list table, found {}", self.tn(&tty))));
                }
                (vec![], Type::list(cap, Type::Int))
            }
            TableOp::Cut => {
                if !self.has_zero(&val) {
                    return Err(self.diag(format!("{what} can't reset a slot of type {}: it has no zero value", self.tn(&val))));
                }
                (vec![key.clone()], Type::Void)
            }
            TableOp::Get => (vec![key.clone()], val.clone()),
            TableOp::Set => (vec![key.clone(), val.clone()], Type::Void),
            TableOp::Has => (vec![key.clone()], Type::Bool),
            TableOp::Push => {
                if !is_list { return Err(list_only(self, "; use .pos(set, key, value) to add entries")); }
                (vec![val.clone()], Type::Void)
            }
            TableOp::Pop => {
                if !is_list { return Err(list_only(self, "")); }
                (vec![], val.clone())
            }
            TableOp::Clear => (vec![], Type::Void),
            TableOp::IsEmpty => (vec![], Type::Bool),
            TableOp::Keys => (vec![], Type::list(cap, key.clone())),
            TableOp::Vals => (vec![], Type::list(cap, val.clone())),
            TableOp::Count => (vec![val.clone()], Type::Int),
            TableOp::Contains => (vec![val.clone()], Type::Bool),
            TableOp::Find => {
                if !is_list { return Err(list_only(self, "; use .pos(has, key) to look up keys")); }
                (vec![val.clone()], Type::Int)
            }
            TableOp::Min | TableOp::Max => match &val {
                Type::Int | Type::Float | Type::Str => (vec![], val.clone()),
                _ => return Err(self.diag(format!("{what} needs a table of i, f or s values, found {}", self.tn(&tty)))),
            },
            TableOp::First | TableOp::Last | TableOp::PopFront | TableOp::Pick => {
                if !is_list { return Err(list_only(self, "")); }
                (vec![], val.clone())
            }
            TableOp::GetOr => (vec![key.clone(), val.clone()], val.clone()),
            TableOp::Insert => {
                if !is_list { return Err(list_only(self, "; use .pos(set, key, value) to add entries")); }
                (vec![Type::Int, val.clone()], Type::Void)
            }
            TableOp::Remove => (vec![key.clone()], val.clone()),
            TableOp::SwapRemove => {
                if !is_list { return Err(list_only(self, "; use .edit(remove, key)")); }
                (vec![Type::Int], val.clone())
            }
            TableOp::Extend => (vec![tty.clone()], Type::Void),
            TableOp::Sort => {
                if is_list && !matches!(val, Type::Int | Type::Float | Type::Str | Type::Bool) {
                    return Err(self.diag(format!(
                        "{what} needs a list of i, f, s or bool; to sort {} use .edit(sort_by, lambda(x: {}) -> f {{ ... }})",
                        self.tn(&tty), self.tn(&val))));
                }
                (vec![], Type::Void)
            }
            TableOp::Reverse => (vec![], Type::Void),
            TableOp::Shuffle => {
                if !is_list { return Err(list_only(self, "")); }
                (vec![], Type::Void)
            }
            TableOp::Copy => (vec![], tty.clone()),
            TableOp::Slice => {
                if !is_list { return Err(list_only(self, "")); }
                (vec![Type::Int, Type::Int], tty.clone())
            }
            TableOp::Join => {
                if !is_list { return Err(list_only(self, "; join its .view(vals) or .view(keys)")); }
                (vec![Type::Str], Type::Str)
            }
            TableOp::SortBy | TableOp::Filter => {
                if !is_list { return Err(list_only(self, "")); }
                if rest.len() != 1 {
                    return Err(self.diag(format!("{what} expects 1 argument (a function), got {}", rest.len())));
                }
                let f = self.expr(&rest[0], None)?;
                let ok_ret = |r: &Type| if op == TableOp::Filter { *r == Type::Bool } else { matches!(r, Type::Int | Type::Float | Type::Str) };
                let want = if op == TableOp::Filter { "bool" } else { "i, f or s" };
                match &f.ty {
                    Type::Func { params, ret } if params.len() == 1 && params[0] == val && ok_ret(ret) => {}
                    other => return Err(self.diag(format!(
                        "{what} needs a function fn({}) -> {want}, found {}", self.tn(&val), self.tn(other)))),
                }
                let ret = if op == TableOp::Filter { tty.clone() } else { Type::Void };
                return Ok(Expr { span: Span::default(), kind: ExprKind::Table { op, table: Box::new(table), args: vec![f] }, ty: ret });
            }
        };
        let checked = self.args_for(&what, &params, rest)?;
        Ok(Expr { span: Span::default(), kind: ExprKind::Table { op, table: Box::new(table), args: checked }, ty: ret })
    }

    fn lambda(&mut self, params: &[ast::Param], ret_ty: &ast::TypeExpr, body: &ast::Block) -> R<Expr> {
        let (ps, ret) = self.resolve_sig(params, ret_ty)?;
        let id = self.alloc_func("<lambda>", ir::FuncKind::Lambda, self.span);
        self.prog.functions[id as usize].params = ps.clone();
        self.prog.functions[id as usize].ret = ret.clone();
        self.push_fn(id, true);
        for (p, t) in params.iter().zip(ps.iter()) {
            self.declare_local(&p.name, t.clone(), true)?;
        }
        let span = self.span;
        let stmts = self.block_stmts(body)?;
        self.span = span;
        if ret != Type::Void && !returns(&stmts) {
            self.fstack.pop();
            return Err(self.diag("lambda doesn't return a value on every path".into()));
        }
        let captures: Vec<UpvalSource> = self.fstack.last().unwrap().func.upvalues.iter().map(|u| u.source).collect();
        self.pop_fn(id, stmts);
        Ok(Expr { span: Span::default(), kind: ExprKind::Closure { func: id, captures }, ty: func_type(ps, ret) })
    }

    // ------------------------------------------------------------- errors

    fn tn(&self, t: &Type) -> String { self.prog.type_name(t) }

    fn diag(&self, message: String) -> Diagnostic { self.diag_at(self.file, self.span, message) }

    fn diag_at(&self, file: ir::FileId, span: Span, message: String) -> Diagnostic {
        let f = &self.prog.files[file as usize];
        let (line, col) = skol_syntax::line_col(&f.source, span.start);
        Diagnostic { message, file: f.path.clone(), line, col }
    }
}

// ------------------------------------------------------------------ helpers

/// A folded constant as an IR literal of the declared type.
fn const_as(v: ConstVal, ty: &Type) -> Option<ExprKind> {
    Some(match (v, ty) {
        (ConstVal::I(n), Type::Int) => ExprKind::Int(n),
        (ConstVal::I(n), Type::Float) => ExprKind::Float(n as f64),
        (ConstVal::F(x), Type::Float) => ExprKind::Float(x),
        (ConstVal::S(t), Type::Str) => ExprKind::Str(t.into()),
        (ConstVal::B(b), Type::Bool) => ExprKind::Bool(b),
        _ => return None,
    })
}

fn func_type(params: Vec<Type>, ret: Type) -> Type { Type::Func { params: params.into(), ret: Rc::new(ret) } }

fn local(l: ir::LocalId, ty: Type) -> Expr { Expr { span: Span::default(), kind: ExprKind::Local(l), ty } }

fn un(op: UnOp, e: Expr, ty: Type) -> Expr { Expr { span: Span::default(), kind: ExprKind::Unary(op, Box::new(e)), ty } }

fn vec_index(field: &str, n: u8) -> Option<u8> {
    let i = match field { "x" => 0, "y" => 1, "z" => 2, "w" => 3, _ => return None };
    if i < n { Some(i) } else { None }
}

/// Does this statement list return on every path?
fn returns(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match &s.kind {
        StmtKind::Return(_) => true,
        StmtKind::If { then_body, else_body, .. } => returns(then_body) && returns(else_body),
        _ => false,
    })
}

/// REPL: print the value of a top-level expression statement.
fn echo(st: Stmt) -> Stmt {
    match st.kind {
        StmtKind::Expr(e) if e.ty != Type::Void => Stmt {
            kind: StmtKind::Expr(Expr { span: Span::default(), kind: ExprKind::CallBuiltin { builtin: ir::Builtin::Print, args: vec![e] }, ty: Type::Void }),
            span: st.span,
        },
        other => Stmt { kind: other, span: st.span },
    }
}
