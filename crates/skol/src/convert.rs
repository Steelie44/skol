//! Converting between Rust values and Skol values, and turning ordinary Rust
//! closures into typed host functions.

use skol_ir::Type;
use skol_stdlib::{Value, VecN};

/// A 2D vector as seen from Rust (`vec2` in Skol).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 { pub x: f64, pub y: f64 }
/// A 3D vector (`vec3` in Skol).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 { pub x: f64, pub y: f64, pub z: f64 }
/// A 4D vector (`vec4` in Skol).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec4 { pub x: f64, pub y: f64, pub z: f64, pub w: f64 }

impl Vec2 { pub fn new(x: f64, y: f64) -> Self { Vec2 { x, y } } }
impl Vec3 { pub fn new(x: f64, y: f64, z: f64) -> Self { Vec3 { x, y, z } } }
impl Vec4 { pub fn new(x: f64, y: f64, z: f64, w: f64) -> Self { Vec4 { x, y, z, w } } }

/// A Rust type with a Skol counterpart.
pub trait SkolType: Sized {
    /// The Skol type it maps to.
    fn skol_type() -> Type;
    /// Convert from a Skol value (None if the value has another type).
    fn from_value(v: &Value) -> Option<Self>;
    fn into_value(self) -> Value;
}

impl SkolType for i64 {
    fn skol_type() -> Type { Type::Int }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Int(i) = v { Some(*i) } else { None } }
    fn into_value(self) -> Value { Value::Int(self) }
}
impl SkolType for i32 {
    fn skol_type() -> Type { Type::Int }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Int(i) = v { i32::try_from(*i).ok() } else { None } }
    fn into_value(self) -> Value { Value::Int(self as i64) }
}
impl SkolType for f64 {
    fn skol_type() -> Type { Type::Float }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Float(x) = v { Some(*x) } else { None } }
    fn into_value(self) -> Value { Value::Float(self) }
}
impl SkolType for f32 {
    fn skol_type() -> Type { Type::Float }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Float(x) = v { Some(*x as f32) } else { None } }
    fn into_value(self) -> Value { Value::Float(self as f64) }
}
impl SkolType for bool {
    fn skol_type() -> Type { Type::Bool }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Bool(b) = v { Some(*b) } else { None } }
    fn into_value(self) -> Value { Value::Bool(self) }
}
impl SkolType for String {
    fn skol_type() -> Type { Type::Str }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Str(s) = v { Some(s.to_string()) } else { None } }
    fn into_value(self) -> Value { Value::str(&self) }
}
impl SkolType for () {
    fn skol_type() -> Type { Type::Void }
    fn from_value(v: &Value) -> Option<Self> { if let Value::Void = v { Some(()) } else { None } }
    fn into_value(self) -> Value { Value::Void }
}
impl SkolType for Vec2 {
    fn skol_type() -> Type { Type::Vec(2) }
    fn from_value(v: &Value) -> Option<Self> { v.as_vec().filter(|v| v.n == 2).map(|v| Vec2::new(v.c[0], v.c[1])) }
    fn into_value(self) -> Value { Value::vec(VecN { n: 2, c: [self.x, self.y, 0.0, 0.0] }) }
}
impl SkolType for Vec3 {
    fn skol_type() -> Type { Type::Vec(3) }
    fn from_value(v: &Value) -> Option<Self> { v.as_vec().filter(|v| v.n == 3).map(|v| Vec3::new(v.c[0], v.c[1], v.c[2])) }
    fn into_value(self) -> Value { Value::vec(VecN { n: 3, c: [self.x, self.y, self.z, 0.0] }) }
}
impl SkolType for Vec4 {
    fn skol_type() -> Type { Type::Vec(4) }
    fn from_value(v: &Value) -> Option<Self> { v.as_vec().filter(|v| v.n == 4).map(|v| Vec4::new(v.c[0], v.c[1], v.c[2], v.c[3])) }
    fn into_value(self) -> Value { Value::vec(VecN { n: 4, c: [self.x, self.y, self.z, self.w] }) }
}

/// What a host function may return: a Skol-typed value, or a `Result` whose
/// `Err` becomes a runtime error in the script (with its stack trace).
pub trait HostReturn {
    fn ret_type() -> Type;
    fn into_result(self) -> Result<Value, String>;
}
impl<T: SkolType> HostReturn for T {
    fn ret_type() -> Type { T::skol_type() }
    fn into_result(self) -> Result<Value, String> { Ok(self.into_value()) }
}
impl<T: SkolType> HostReturn for Result<T, String> {
    fn ret_type() -> Type { T::skol_type() }
    fn into_result(self) -> Result<Value, String> { self.map(SkolType::into_value) }
}

/// A Rust closure usable as a host function. Implemented for closures of up
/// to six `SkolType` arguments returning a `HostReturn`.
pub trait IntoHostFn<Args> {
    fn signature() -> (Vec<Type>, Type);
    fn into_host_fn(self) -> skol_vm::HostFn;
}

macro_rules! impl_into_host_fn {
    ($($arg:ident),*) => {
        impl<F, Ret, $($arg),*> IntoHostFn<($($arg,)*)> for F
        where
            F: FnMut($($arg),*) -> Ret + 'static,
            Ret: HostReturn,
            $($arg: SkolType),*
        {
            fn signature() -> (Vec<Type>, Type) { (vec![$($arg::skol_type()),*], Ret::ret_type()) }

            #[allow(non_snake_case, unused_mut, unused_variables, unused_assignments)]
            fn into_host_fn(mut self) -> skol_vm::HostFn {
                Box::new(move |args: &[Value]| {
                    let mut it = args.iter();
                    $(
                        let $arg = match it.next().and_then(|v| $arg::from_value(v)) {
                            Some(x) => x,
                            None => return Err("host function received an argument of the wrong type".to_string()),
                        };
                    )*
                    (self)($($arg),*).into_result()
                })
            }
        }
    };
}

impl_into_host_fn!();
impl_into_host_fn!(A);
impl_into_host_fn!(A, B);
impl_into_host_fn!(A, B, C);
impl_into_host_fn!(A, B, C, D);
impl_into_host_fn!(A, B, C, D, E);
impl_into_host_fn!(A, B, C, D, E, G);

/// Arguments for calling a script function from Rust: a tuple of up to six
/// `SkolType` values, e.g. `(0.016,)` or `("ada".to_string(), 3)`.
pub trait IntoArgs {
    fn types() -> Vec<Type>;
    fn into_values(self) -> Vec<Value>;
}

macro_rules! impl_into_args {
    ($($arg:ident),*) => {
        impl<$($arg: SkolType),*> IntoArgs for ($($arg,)*) {
            fn types() -> Vec<Type> { vec![$($arg::skol_type()),*] }
            #[allow(non_snake_case)]
            fn into_values(self) -> Vec<Value> {
                let ($($arg,)*) = self;
                vec![$($arg.into_value()),*]
            }
        }
    };
}

impl_into_args!();
impl_into_args!(A);
impl_into_args!(A, B);
impl_into_args!(A, B, C);
impl_into_args!(A, B, C, D);
impl_into_args!(A, B, C, D, E);
impl_into_args!(A, B, C, D, E, G);

/// A value whose Skol type is decided at run time (for `Engine::call_dynamic`).
#[derive(Debug, Clone, PartialEq)]
pub enum Dyn {
    Void,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
}

impl Dyn {
    pub fn type_name(&self) -> &'static str {
        match self {
            Dyn::Void => "void", Dyn::Int(_) => "i", Dyn::Float(_) => "f", Dyn::Bool(_) => "bool",
            Dyn::Str(_) => "s", Dyn::Vec2(_) => "vec2", Dyn::Vec3(_) => "vec3", Dyn::Vec4(_) => "vec4",
        }
    }

    /// Convert to a Skol value of type `ty` (an `Int` widens to `f`).
    pub(crate) fn to_value(&self, ty: &Type) -> Option<Value> {
        Some(match (self, ty) {
            (Dyn::Int(i), Type::Int) => Value::Int(*i),
            (Dyn::Int(i), Type::Float) => Value::Float(*i as f64),
            (Dyn::Float(x), Type::Float) => Value::Float(*x),
            (Dyn::Bool(b), Type::Bool) => Value::Bool(*b),
            (Dyn::Str(s), Type::Str) => Value::str(s),
            (Dyn::Vec2(v), Type::Vec(2)) => Value::Vec2(v.x, v.y),
            (Dyn::Vec3(v), Type::Vec(3)) => Value::vec(VecN { n: 3, c: [v.x, v.y, v.z, 0.0] }),
            (Dyn::Vec4(v), Type::Vec(4)) => Value::vec(VecN { n: 4, c: [v.x, v.y, v.z, v.w] }),
            _ => return None,
        })
    }

    pub(crate) fn from_value(v: &Value) -> Option<Dyn> {
        Some(match v {
            Value::Void => Dyn::Void,
            Value::Int(i) => Dyn::Int(*i),
            Value::Float(x) => Dyn::Float(*x),
            Value::Bool(b) => Dyn::Bool(*b),
            Value::Str(s) => Dyn::Str(s.to_string()),
            Value::Vec2(x, y) => Dyn::Vec2(Vec2::new(*x, *y)),
            Value::Vec(n) if n.n == 3 => Dyn::Vec3(Vec3::new(n.c[0], n.c[1], n.c[2])),
            Value::Vec(n) if n.n == 4 => Dyn::Vec4(Vec4::new(n.c[0], n.c[1], n.c[2], n.c[3])),
            _ => return None,
        })
    }
}
