//! Typed intermediate representation for Skol.
//!
//! `skol-sema` lowers the untyped AST into this form after resolving every
//! name and checking every type. Nothing here is ambiguous: each variable
//! reference says whether it is a local slot, a captured upvalue or a global;
//! each operator is specialised to its operand types; each call says exactly
//! what it calls. That makes the IR the single source of truth for language
//! semantics — the bytecode compiler in `skol-vm` consumes it today, and an
//! ahead-of-time C++/Rust backend can consume the same structures later.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

pub use skol_syntax::Span;

pub type FuncId = u32;
pub type ClassId = u32;
pub type CompId = u32;
pub type GlobalId = u32;
pub type SignalId = u32;
pub type FileId = u32;
pub type LocalId = u16;

// ---------------------------------------------------------------- types

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Str,
    Bool,
    Void,
    /// vec2 / vec3 / vec4 — value types of `f` components.
    Vec(u8),
    /// `T<cap>[elem]` sequential table.
    List { cap: usize, elem: Rc<Type> },
    /// `T<cap>[key, val]` key/value table. Keys are `i`, `s` or `bool`.
    Map { cap: usize, key: Rc<Type>, val: Rc<Type> },
    Func { params: Rc<[Type]>, ret: Rc<Type> },
    Class(ClassId),
    /// A component used as a type: accepts any instance whose class includes it.
    Comp(CompId),
    Signal(SignalId),
    /// Result of `range(...)`; only usable in `for` loops.
    Range,
    /// Only for built-in parameters (print, str) and untyped signal params.
    Any,
}

impl Type {
    pub fn list(cap: usize, elem: Type) -> Type { Type::List { cap, elem: Rc::new(elem) } }
    pub fn is_numeric(&self) -> bool { matches!(self, Type::Int | Type::Float) }
}

/// Names needed to print types that refer to classes, components and signals.
pub trait TypeNames {
    fn class_name(&self, id: ClassId) -> &str;
    fn comp_name(&self, id: CompId) -> &str;
    fn signal_name(&self, id: SignalId) -> &str;
}

pub struct DisplayType<'a, N: TypeNames + ?Sized>(pub &'a Type, pub &'a N);

impl<N: TypeNames + ?Sized> fmt::Display for DisplayType<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.1;
        match self.0 {
            Type::Int => write!(f, "i"),
            Type::Float => write!(f, "f"),
            Type::Str => write!(f, "s"),
            Type::Bool => write!(f, "bool"),
            Type::Void => write!(f, "void"),
            Type::Vec(k) => write!(f, "vec{k}"),
            Type::List { cap, elem } => write!(f, "T{cap}[{}]", DisplayType(elem, n)),
            Type::Map { cap, key, val } => write!(f, "T{cap}[{}, {}]", DisplayType(key, n), DisplayType(val, n)),
            Type::Func { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", DisplayType(p, n))?;
                }
                write!(f, ") -> {}", DisplayType(ret, n))
            }
            Type::Class(c) => write!(f, "{}", n.class_name(*c)),
            Type::Comp(c) => write!(f, "{}", n.comp_name(*c)),
            Type::Signal(s) => write!(f, "signal {}", n.signal_name(*s)),
            Type::Range => write!(f, "range"),
            Type::Any => write!(f, "any"),
        }
    }
}

// ---------------------------------------------------------------- program

#[derive(Debug, Clone, Default)]
pub struct Program {
    pub files: Vec<SourceFile>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    pub classes: Vec<Class>,
    pub comps: Vec<Comp>,
    pub signals: Vec<Signal>,
    /// Module initialisers in dependency order (imports first).
    pub module_inits: Vec<FuncId>,
    /// `fn main()` of the entry module, called after all initialisers.
    pub main: Option<FuncId>,
}

impl TypeNames for Program {
    fn class_name(&self, id: ClassId) -> &str { &self.classes[id as usize].name }
    fn comp_name(&self, id: CompId) -> &str { &self.comps[id as usize].name }
    fn signal_name(&self, id: SignalId) -> &str { &self.signals[id as usize].name }
}

impl Program {
    pub fn type_name(&self, ty: &Type) -> String { DisplayType(ty, self).to_string() }
}

#[derive(Debug, Clone)]
pub struct SourceFile { pub path: String, pub source: Rc<str> }

#[derive(Debug, Clone)]
pub struct Global { pub name: String, pub ty: Type, pub mutable: bool }

#[derive(Debug, Clone)]
pub struct Class {
    pub name: String,
    pub comps: Vec<CompId>,
    /// Flattened layout: component fields first (in declaration order), then
    /// the class's own fields.
    pub fields: Vec<Field>,
    pub field_index: HashMap<String, u32>,
    /// Method name -> implementation (component methods are specialised per class).
    pub methods: HashMap<String, FuncId>,
    /// `ClassName(args)`: allocates, applies field defaults, calls `init`.
    pub ctor: FuncId,
    /// Allocates and applies field defaults only (used for zero values).
    pub default_ctor: FuncId,
}

#[derive(Debug, Clone)]
pub struct Field { pub name: String, pub ty: Type, pub from_comp: Option<CompId> }

#[derive(Debug, Clone)]
pub struct Comp {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    /// Method signatures (excluding `self`), used for calls through a `Comp` type.
    pub methods: HashMap<String, Type>,
}

#[derive(Debug, Clone)]
pub struct Signal { pub name: String, pub params: Vec<String>, pub global: GlobalId }

#[derive(Debug, Clone, PartialEq)]
pub enum FuncKind {
    Function,
    Method { class: ClassId },
    Lambda,
    ModuleInit,
    Ctor { class: ClassId },
    Defaults { class: ClassId },
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub kind: FuncKind,
    /// Parameter types; for methods the first is `self`.
    pub params: Vec<Type>,
    pub ret: Type,
    /// All local slots. Parameters occupy the first `params.len()` slots.
    pub locals: Vec<Local>,
    pub upvalues: Vec<Upvalue>,
    pub body: Vec<Stmt>,
    pub file: FileId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub ty: Type,
    /// Captured by a closure: lives in a heap cell so mutations are shared.
    pub captured: bool,
}

#[derive(Debug, Clone)]
pub struct Upvalue { pub name: String, pub ty: Type, pub source: UpvalSource }

/// Where a closure's upvalue comes from, relative to the function that
/// creates the closure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpvalSource { Local(LocalId), Upval(u16) }

// ---------------------------------------------------------------- statements

#[derive(Debug, Clone)]
pub struct Stmt { pub kind: StmtKind, pub span: Span }

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { local: LocalId, value: Expr },
    /// `place = value` or, with `op`, `place = place <op> value`.
    Assign { place: Place, op: Option<BinOp>, value: Expr },
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    /// `iter` and `index` are hidden locals holding the iterable and cursor.
    For { var: LocalId, iter: LocalId, index: LocalId, iterable: Expr, body: Vec<Stmt> },
    Return(Option<Expr>),
    /// Exit the innermost loop.
    Break,
    /// Jump to the next iteration of the innermost loop.
    Continue,
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Place {
    Local(LocalId),
    Global(GlobalId),
    Upval(u16),
    Field { obj: Box<Expr>, slot: u32 },
    /// Field of a component-typed value; resolved by name at runtime.
    FieldNamed { obj: Box<Expr>, name: Rc<str> },
    /// `v.x = ...` on a vector stored in `base`.
    VecComp { base: Box<Place>, index: u8 },
    /// `table[key] = ...` (list index or map key).
    TableSlot { table: Box<Expr>, key: Box<Expr> },
}

// ---------------------------------------------------------------- expressions

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    /// Source span of the expression (default when synthesised by the checker).
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Type) -> Expr { Expr { kind, ty, span: Span::default() } }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Bool(bool),
    Void,
    Local(LocalId),
    Global(GlobalId),
    Upval(u16),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    IntToFloat(Box<Expr>),
    MakeVec(Vec<Expr>),
    MakeList(Vec<Expr>),
    MakeMap(Vec<(Expr, Expr)>),
    /// The zero/default value of `self.ty`.
    Zero,
    NewInstance(ClassId),
    NewSignal(SignalId),
    GetField { obj: Box<Expr>, slot: u32 },
    GetFieldNamed { obj: Box<Expr>, name: Rc<str> },
    VecComp { vec: Box<Expr>, index: u8 },
    /// Direct call; for methods the receiver is `args[0]`.
    CallFunc { func: FuncId, args: Vec<Expr> },
    CallMethodNamed { recv: Box<Expr>, name: Rc<str>, args: Vec<Expr> },
    CallValue { callee: Box<Expr>, args: Vec<Expr> },
    CallBuiltin { builtin: Builtin, args: Vec<Expr> },
    /// A function value with captured upvalues (empty for plain `fn` refs).
    Closure { func: FuncId, captures: Vec<UpvalSource> },
    BindMethod { recv: Box<Expr>, func: FuncId },
    BindMethodNamed { recv: Box<Expr>, name: Rc<str> },
    Table { op: TableOp, table: Box<Expr>, args: Vec<Expr> },
    SignalListen { signal: Box<Expr>, handler: Box<Expr> },
    SignalEmit { signal: Box<Expr>, args: Vec<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp { NegI, NegF, NegV, Not }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp { Lt, Le, Gt, Ge }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    AddI, SubI, MulI, DivI, ModI,
    AddF, SubF, MulF, DivF, ModF,
    Concat,
    VecAdd, VecSub, VecMul,
    /// vec * f
    VecScale,
    /// f * vec
    ScaleVec,
    /// vec / f
    VecDiv,
    CmpI(Cmp), CmpF(Cmp), CmpS(Cmp),
    Eq, Ne,
}

/// The four v0.1 table dispatchers and their operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableOp {
    // .iter(...)
    Len, Sum, Even, Cut, Count, Find, Contains, Min, Max,
    // .pos(...)
    Get, Set, Has, First, Last, GetOr,
    // .edit(...)
    Push, Pop, Clear, Insert, Remove, SwapRemove, PopFront, Extend, Sort, SortBy, Reverse, Shuffle,
    // .view(...)
    IsEmpty, Keys, Vals, Copy, Slice, Join, Filter, Pick,
}

impl TableOp {
    pub const ALL: &'static [TableOp] = &[
        TableOp::Len, TableOp::Sum, TableOp::Even, TableOp::Cut, TableOp::Count, TableOp::Find,
        TableOp::Contains, TableOp::Min, TableOp::Max,
        TableOp::Get, TableOp::Set, TableOp::Has, TableOp::First, TableOp::Last, TableOp::GetOr,
        TableOp::Push, TableOp::Pop, TableOp::Clear, TableOp::Insert, TableOp::Remove, TableOp::SwapRemove,
        TableOp::PopFront, TableOp::Extend, TableOp::Sort, TableOp::SortBy, TableOp::Reverse, TableOp::Shuffle,
        TableOp::IsEmpty, TableOp::Keys, TableOp::Vals, TableOp::Copy, TableOp::Slice, TableOp::Join,
        TableOp::Filter, TableOp::Pick,
    ];
    pub fn dispatcher(self) -> &'static str {
        use TableOp::*;
        match self {
            Len | Sum | Even | Cut | Count | Find | Contains | Min | Max => "iter",
            Get | Set | Has | First | Last | GetOr => "pos",
            Push | Pop | Clear | Insert | Remove | SwapRemove | PopFront | Extend | Sort | SortBy | Reverse | Shuffle => "edit",
            IsEmpty | Keys | Vals | Copy | Slice | Join | Filter | Pick => "view",
        }
    }
    pub fn name(self) -> &'static str {
        use TableOp::*;
        match self {
            Len => "len", Sum => "sum", Even => "even", Cut => "cut", Count => "count", Find => "find",
            Contains => "contains", Min => "min", Max => "max",
            Get => "get", Set => "set", Has => "has", First => "first", Last => "last", GetOr => "get_or",
            Push => "push", Pop => "pop", Clear => "clear", Insert => "insert", Remove => "remove",
            SwapRemove => "swap_remove", PopFront => "pop_front", Extend => "extend", Sort => "sort",
            SortBy => "sort_by", Reverse => "reverse", Shuffle => "shuffle",
            IsEmpty => "is_empty", Keys => "keys", Vals => "vals", Copy => "copy", Slice => "slice",
            Join => "join", Filter => "filter", Pick => "pick",
        }
    }
    pub fn lookup(dispatcher: &str, op: &str) -> Option<TableOp> {
        Self::ALL.iter().copied().find(|t| t.dispatcher() == dispatcher && t.name() == op)
    }
    pub fn ops_for(dispatcher: &str) -> Vec<&'static str> {
        Self::ALL.iter().filter(|t| t.dispatcher() == dispatcher).map(|t| t.name()).collect()
    }
}

/// Built-in functions. Signatures live in skol-sema; implementations in
/// skol-stdlib.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Builtin {
    // prelude
    Print, ToStr, TypeOf, Assert, AssertMsg, Error,
    FloatToInt, StrToInt, IntToFloat, StrToFloat,
    Range1, Range2,
    Input, InputPrompt,
    /// `fmt(template, args...)`: `{}` placeholders, `{.N}` fixed decimals.
    Fmt,
    /// `chr(code)`: the one-character string for a Unicode code point.
    Chr,
    // io module
    ReadFile, WriteFile, AppendFile, FileExists,
    /// `read_lines(path)`; the checker appends the destination capacity.
    ReadLines,
    /// `list_dir(path)`; the checker appends the destination capacity.
    ListDir,
    DeleteFile, DeleteDir, MakeDir, DirExists, RenameFile,
    // math module
    Sqrt, Sin, Cos, Tan, Asin, Acos, Atan, Atan2, Pow, Exp, Log, Log10,
    AbsF, AbsI, SignF, SignI, Floor, Ceil, Round,
    MinF, MaxF, MinI, MaxI, ClampF, ClampI, Lerp, InverseLerp, Remap, Smoothstep, MoveToward,
    WrapI, WrapF, Snapped, DegToRad, RadToDeg, Mod, Random, RandomInt, Time,
    // random module
    Seed, RandF, RandI, RandIRange, RandFRange, Chance, RandDir,
    // time module
    TicksMsec, TicksUsec, Now, Sleep, Date0, Date1, DateString0, DateString1,
    // os module
    ArgCount, Arg, Env, HasEnv, Platform, Exit,
    // string methods (receiver is args[0])
    StrLen, StrUpper, StrLower, StrContains, StrStartsWith, StrEndsWith, StrTrim,
    StrReplace, StrFind, StrSubstr, StrRepeat,
    StrTrimStart, StrTrimEnd, StrPadLeft, StrPadRight, StrReverse, StrCount, StrRfind,
    StrIsInt, StrIsFloat, StrIsAlpha, StrIsDigit, StrIsSpace, StrCode,
    /// `s[i]`: the character at index i.
    StrAt,
    /// `s.split(sep)`; the checker appends the destination capacity as a third argument.
    StrSplit,
    // vector methods (receiver is args[0])
    VecLength, VecNormalized, VecDot, VecDistance, VecCross,
    /// A function registered by the embedding application (by id).
    Host(u32),
}
