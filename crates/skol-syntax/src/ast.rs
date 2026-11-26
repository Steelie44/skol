use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// Inclusive UTF-8 byte offset in the original source.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the original source.
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self { Self { start, end } }
}

/// A statement or function body. Shared via `Rc` so closures and methods can
/// hold onto their code without cloning the tree.
pub type Block = Rc<Vec<Stmt>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Program { pub items: Vec<Item> }

#[derive(Debug, Clone, PartialEq)]
pub struct Item { pub kind: ItemKind, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Import(ImportDecl),
    Binding(BindingDecl),
    Fn(Rc<FnDecl>),
    Class(Rc<ClassDecl>),
    Component(Rc<ComponentDecl>),
    Signal(SignalDecl),
    Stmt(Stmt),
}

/// `from Components use Movement, Health`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl { pub module: String, pub names: Vec<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct BindingDecl {
    pub mutable: bool,
    pub name: String,
    pub ty: TypeExpr,
    pub init: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param { pub name: String, pub ty: TypeExpr }

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_ty: TypeExpr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl { pub name: String, pub ty: TypeExpr, pub default: Option<Expr>, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub components: Vec<String>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<Rc<FnDecl>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDecl {
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<Rc<FnDecl>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalDecl { pub name: String, pub params: Vec<String> }

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Int, Float, Str, Bool, Void,
    Vec2, Vec3, Vec4,
    /// Legacy unbounded forms retained temporarily for parser migration.
    Table1D(Box<TypeExpr>),
    Table2D(Box<TypeExpr>, Box<TypeExpr>),
    /// Fixed-capacity sequential table.
    FixedTable1D { capacity: usize, element: Box<TypeExpr> },
    /// Fixed-capacity key/value table.
    FixedTable2D { capacity: usize, key: Box<TypeExpr>, value: Box<TypeExpr> },
    Function { params: Vec<TypeExpr>, ret: Box<TypeExpr> },
    Named(String),
}

impl std::fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeExpr::Int => write!(f, "i"),
            TypeExpr::Float => write!(f, "f"),
            TypeExpr::Str => write!(f, "s"),
            TypeExpr::Bool => write!(f, "bool"),
            TypeExpr::Void => write!(f, "void"),
            TypeExpr::Vec2 => write!(f, "vec2"),
            TypeExpr::Vec3 => write!(f, "vec3"),
            TypeExpr::Vec4 => write!(f, "vec4"),
            TypeExpr::Table1D(e) => write!(f, "T[{e}]"),
            TypeExpr::Table2D(k, v) => write!(f, "T[{k}, {v}]"),
            TypeExpr::FixedTable1D { capacity, element } => write!(f, "T{capacity}[{element}]"),
            TypeExpr::FixedTable2D { capacity, key, value } => write!(f, "T{capacity}[{key}, {value}]"),
            TypeExpr::Function { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeExpr::Named(n) => write!(f, "{n}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt { pub kind: StmtKind, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Binding(BindingDecl),
    If { cond: Expr, then_body: Block, else_body: Option<Block> },
    For { var: String, iterable: Expr, body: Block },
    While { cond: Expr, body: Block },
    Return(Option<Expr>),
    Pass,
    Break,
    Continue,
    Assign { target: Expr, op: AssignOp, value: Expr },
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp { Assign, PlusAssign, MinusAssign }

/// An expression with the source span it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr { pub kind: ExprKind, pub span: Span }

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    IntLit(i64), FloatLit(f64), StrLit(String), BoolLit(bool), Ident(String),
    TableLit(Vec<TableEntry>),
    Unary { op: UnaryOp, expr: Box<Expr> },
    Binary { left: Box<Expr>, op: BinaryOp, right: Box<Expr> },
    Field { object: Box<Expr>, field: String },
    /// `object[index]`
    Index { object: Box<Expr>, index: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    MethodCall { object: Box<Expr>, method: String, args: Vec<Expr> },
    /// Typed lambda expression. `:=` is reserved for lambda binding syntax.
    Lambda { params: Vec<Param>, ret_ty: TypeExpr, body: Block },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableEntry { Item(Expr), Pair(Expr, Expr) }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp { Neg, Not }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp { Add, Sub, Mul, Div, Mod, Eq, NotEq, Lt, LtEq, Gt, GtEq, And, Or }
