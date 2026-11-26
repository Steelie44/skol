//! Signatures of built-in functions and methods. Implementations live in
//! skol-stdlib; this file only decides which `Builtin` a call means and
//! what types it takes and returns.

use skol_ir::{Builtin, Type};

/// Functions available everywhere without an import.
pub const PRELUDE: &[&str] = &["print", "str", "int", "float", "type_of", "assert", "range", "error", "input"];

/// Names exported by the built-in `io` module (`from io use read_file`).
pub fn io_symbol(name: &str) -> Option<&'static str> {
    ["read_file", "write_file", "append_file", "file_exists"].into_iter().find(|f| *f == name)
}

/// Names exported by the built-in `math` module (`from math use sqrt, pi`).
pub enum MathSym { Func(&'static str), Const(f64) }

pub fn math_symbol(name: &str) -> Option<MathSym> {
    const FUNCS: &[&str] = &[
        "sqrt", "sin", "cos", "tan", "atan2", "pow", "abs", "floor", "ceil", "round",
        "min", "max", "clamp", "lerp", "mod", "random", "random_int", "time",
    ];
    match name {
        "pi" => Some(MathSym::Const(std::f64::consts::PI)),
        "tau" => Some(MathSym::Const(std::f64::consts::TAU)),
        "e" => Some(MathSym::Const(std::f64::consts::E)),
        _ => FUNCS.iter().find(|f| **f == name).map(|f| MathSym::Func(f)),
    }
}

pub enum Resolved {
    /// Call `builtin` after coercing each argument to `params`.
    Call { builtin: Builtin, params: Vec<Type>, ret: Type },
    /// Any number of arguments of any non-void type.
    Variadic { builtin: Builtin, ret: Type },
    /// Conversion to the same type: the argument is returned unchanged.
    Identity,
}

fn call(builtin: Builtin, params: Vec<Type>, ret: Type) -> Result<Resolved, String> {
    Ok(Resolved::Call { builtin, params, ret })
}

fn arity(name: &str, args: &[Type], n: usize) -> Result<(), String> {
    if args.len() == n { Ok(()) } else {
        Err(format!("{name}() expects {n} argument{}, got {}", if n == 1 { "" } else { "s" }, args.len()))
    }
}

/// Resolve a call to a prelude or math function given the argument types.
pub fn resolve_function(name: &str, args: &[Type]) -> Result<Resolved, String> {
    use Type::*;
    let all_int = |a: &[Type]| a.iter().all(|t| *t == Int);
    match name {
        "print" => Ok(Resolved::Variadic { builtin: Builtin::Print, ret: Void }),
        "str" => { arity(name, args, 1)?; call(Builtin::ToStr, vec![Any], Str) }
        "type_of" => { arity(name, args, 1)?; call(Builtin::TypeOf, vec![Any], Str) }
        "int" => {
            arity(name, args, 1)?;
            match args[0] {
                Int => Ok(Resolved::Identity),
                Float => call(Builtin::FloatToInt, vec![Float], Int),
                Str => call(Builtin::StrToInt, vec![Str], Int),
                _ => Err("int() takes an i, f or s".into()),
            }
        }
        "float" => {
            arity(name, args, 1)?;
            match args[0] {
                Float => Ok(Resolved::Identity),
                Int => call(Builtin::IntToFloat, vec![Int], Float),
                Str => call(Builtin::StrToFloat, vec![Str], Float),
                _ => Err("float() takes an i, f or s".into()),
            }
        }
        "assert" => match args.len() {
            1 => call(Builtin::Assert, vec![Bool], Void),
            2 => call(Builtin::AssertMsg, vec![Bool, Str], Void),
            n => Err(format!("assert() expects 1 or 2 arguments, got {n}")),
        },
        "range" => match args.len() {
            1 => call(Builtin::Range1, vec![Int], Range),
            2 => call(Builtin::Range2, vec![Int, Int], Range),
            n => Err(format!("range() expects 1 or 2 arguments, got {n}")),
        },
        "error" => { arity(name, args, 1)?; call(Builtin::Error, vec![Str], Void) }
        "input" => match args.len() {
            0 => call(Builtin::Input, vec![], Str),
            1 => call(Builtin::InputPrompt, vec![Str], Str),
            n => Err(format!("input() expects 0 or 1 arguments, got {n}")),
        },
        // io
        "read_file" => { arity(name, args, 1)?; call(Builtin::ReadFile, vec![Str], Str) }
        "write_file" => { arity(name, args, 2)?; call(Builtin::WriteFile, vec![Str, Str], Void) }
        "append_file" => { arity(name, args, 2)?; call(Builtin::AppendFile, vec![Str, Str], Void) }
        "file_exists" => { arity(name, args, 1)?; call(Builtin::FileExists, vec![Str], Bool) }
        // math
        "sqrt" | "sin" | "cos" | "tan" => {
            arity(name, args, 1)?;
            let b = match name { "sqrt" => Builtin::Sqrt, "sin" => Builtin::Sin, "cos" => Builtin::Cos, _ => Builtin::Tan };
            call(b, vec![Float], Float)
        }
        "atan2" | "pow" => {
            arity(name, args, 2)?;
            call(if name == "pow" { Builtin::Pow } else { Builtin::Atan2 }, vec![Float, Float], Float)
        }
        "abs" => {
            arity(name, args, 1)?;
            if args[0] == Int { call(Builtin::AbsI, vec![Int], Int) } else { call(Builtin::AbsF, vec![Float], Float) }
        }
        "floor" | "ceil" | "round" => {
            arity(name, args, 1)?;
            let b = match name { "floor" => Builtin::Floor, "ceil" => Builtin::Ceil, _ => Builtin::Round };
            call(b, vec![Float], Int)
        }
        "min" | "max" => {
            arity(name, args, 2)?;
            let int = all_int(args);
            let b = match (name, int) {
                ("min", true) => Builtin::MinI, ("min", false) => Builtin::MinF,
                (_, true) => Builtin::MaxI, _ => Builtin::MaxF,
            };
            if int { call(b, vec![Int, Int], Int) } else { call(b, vec![Float, Float], Float) }
        }
        "clamp" => {
            arity(name, args, 3)?;
            if all_int(args) { call(Builtin::ClampI, vec![Int, Int, Int], Int) }
            else { call(Builtin::ClampF, vec![Float, Float, Float], Float) }
        }
        "lerp" => { arity(name, args, 3)?; call(Builtin::Lerp, vec![Float, Float, Float], Float) }
        "mod" => { arity(name, args, 2)?; call(Builtin::Mod, vec![Int, Int], Int) }
        "random" => { arity(name, args, 0)?; call(Builtin::Random, vec![], Float) }
        "random_int" => { arity(name, args, 2)?; call(Builtin::RandomInt, vec![Int, Int], Int) }
        "time" => { arity(name, args, 0)?; call(Builtin::Time, vec![], Float) }
        _ => Err(format!("unknown built-in '{name}'")),
    }
}

/// Resolve a method on a string or vector receiver. The receiver is passed as
/// the first argument, so `params` includes it.
pub fn resolve_method(recv: &Type, name: &str, args: &[Type]) -> Option<Result<Resolved, String>> {
    use Type::*;
    let m = |b: Builtin, extra: std::vec::Vec<Type>, ret: Type| -> Result<Resolved, String> {
        if args.len() != extra.len() {
            return Err(format!(".{name}() expects {} argument{}, got {}", extra.len(), if extra.len() == 1 { "" } else { "s" }, args.len()));
        }
        let mut params: std::vec::Vec<Type> = vec![recv.clone()];
        params.extend(extra);
        Ok(Resolved::Call { builtin: b, params, ret })
    };
    Some(match (recv, name) {
        (Str, "len") => m(Builtin::StrLen, vec![], Int),
        (Str, "upper") => m(Builtin::StrUpper, vec![], Str),
        (Str, "lower") => m(Builtin::StrLower, vec![], Str),
        (Str, "trim") => m(Builtin::StrTrim, vec![], Str),
        (Str, "contains") => m(Builtin::StrContains, vec![Str], Bool),
        (Str, "starts_with") => m(Builtin::StrStartsWith, vec![Str], Bool),
        (Str, "ends_with") => m(Builtin::StrEndsWith, vec![Str], Bool),
        (Str, "replace") => m(Builtin::StrReplace, vec![Str, Str], Str),
        (Str, "find") => m(Builtin::StrFind, vec![Str], Int),
        (Str, "substr") => m(Builtin::StrSubstr, vec![Int, Int], Str),
        (Str, "repeat") => m(Builtin::StrRepeat, vec![Int], Str),
        (Vec(_), "length") => m(Builtin::VecLength, vec![], Float),
        (Vec(n), "normalized") => m(Builtin::VecNormalized, vec![], Vec(*n)),
        (Vec(n), "dot") => m(Builtin::VecDot, vec![Vec(*n)], Float),
        (Vec(n), "distance") => m(Builtin::VecDistance, vec![Vec(*n)], Float),
        (Vec(3), "cross") => m(Builtin::VecCross, vec![Vec(3)], Vec(3)),
        _ => return None,
    })
}
