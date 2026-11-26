//! Signatures of built-in functions and methods. Implementations live in
//! skol-stdlib; this file only decides which `Builtin` a call means and
//! what types it takes and returns.

use skol_ir::{Builtin, Type};

/// Functions available everywhere without an import.
pub const PRELUDE: &[&str] = &["print", "str", "int", "float", "type_of", "assert", "range", "error", "input", "fmt", "chr"];

/// A name exported by a built-in module.
pub enum MathSym { Func(&'static str), Const(f64) }

const IO: &[&str] = &[
    "read_file", "write_file", "append_file", "file_exists", "read_lines", "list_dir",
    "delete_file", "delete_dir", "make_dir", "dir_exists", "rename_file",
];
const MATH: &[&str] = &[
    "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "pow", "exp", "log", "log10",
    "abs", "sign", "floor", "ceil", "round", "min", "max", "clamp", "lerp", "inverse_lerp", "remap",
    "smoothstep", "move_toward", "wrap", "snapped", "deg_to_rad", "rad_to_deg", "mod",
    "random", "random_int", "time",
];
const MATH_CONSTS: &[&str] = &["pi", "tau", "e", "inf"];
const RANDOM: &[&str] = &["seed", "randf", "randi", "randi_range", "randf_range", "chance", "rand_dir"];
const TIME: &[&str] = &["time", "ticks_msec", "ticks_usec", "now", "sleep", "date", "date_string"];
const OS: &[&str] = &["arg_count", "arg", "env", "has_env", "platform", "exit"];

fn module_table(module: &str) -> Option<&'static [&'static str]> {
    Some(match module { "io" => IO, "math" => MATH, "random" => RANDOM, "time" => TIME, "os" => OS, _ => return None })
}

/// Is this one of the modules built into the runtime?
pub fn is_builtin_module(module: &str) -> bool { module_table(module).is_some() }

/// Every name a built-in module exports (for error messages).
pub fn module_names(module: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = module_table(module).map(|t| t.to_vec()).unwrap_or_default();
    if module == "math" { v.extend_from_slice(MATH_CONSTS); }
    v
}

/// Resolve `from <module> use <name>` for a built-in module.
pub fn module_symbol(module: &str, name: &str) -> Option<MathSym> {
    if module == "math" {
        match name {
            "pi" => return Some(MathSym::Const(std::f64::consts::PI)),
            "tau" => return Some(MathSym::Const(std::f64::consts::TAU)),
            "e" => return Some(MathSym::Const(std::f64::consts::E)),
            "inf" => return Some(MathSym::Const(f64::INFINITY)),
            _ => {}
        }
    }
    module_table(module)?.iter().find(|f| **f == name).map(|f| MathSym::Func(f))
}

/// Built-in functions that build a list and take its capacity from where the
/// result goes (`var files: T64[s] = list_dir("saves")`).
pub fn needs_capacity(name: &str) -> bool { matches!(name, "read_lines" | "list_dir") }

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
        "fmt" => {
            if args.first() != Some(&Str) { return Err("fmt() needs a template string as its first argument, e.g. fmt(\"{} hp\", hp)".into()); }
            Ok(Resolved::Variadic { builtin: Builtin::Fmt, ret: Str })
        }
        "chr" => { arity(name, args, 1)?; call(Builtin::Chr, vec![Int], Str) }
        // io
        "read_file" => { arity(name, args, 1)?; call(Builtin::ReadFile, vec![Str], Str) }
        "write_file" => { arity(name, args, 2)?; call(Builtin::WriteFile, vec![Str, Str], Void) }
        "append_file" => { arity(name, args, 2)?; call(Builtin::AppendFile, vec![Str, Str], Void) }
        "file_exists" => { arity(name, args, 1)?; call(Builtin::FileExists, vec![Str], Bool) }
        "dir_exists" => { arity(name, args, 1)?; call(Builtin::DirExists, vec![Str], Bool) }
        "delete_file" => { arity(name, args, 1)?; call(Builtin::DeleteFile, vec![Str], Void) }
        "delete_dir" => { arity(name, args, 1)?; call(Builtin::DeleteDir, vec![Str], Void) }
        "make_dir" => { arity(name, args, 1)?; call(Builtin::MakeDir, vec![Str], Void) }
        "rename_file" => { arity(name, args, 2)?; call(Builtin::RenameFile, vec![Str, Str], Void) }
        // math
        "sqrt" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "exp" | "log" | "log10" | "deg_to_rad" | "rad_to_deg" => {
            arity(name, args, 1)?;
            let b = match name {
                "sqrt" => Builtin::Sqrt, "sin" => Builtin::Sin, "cos" => Builtin::Cos, "tan" => Builtin::Tan,
                "asin" => Builtin::Asin, "acos" => Builtin::Acos, "atan" => Builtin::Atan, "exp" => Builtin::Exp,
                "log" => Builtin::Log, "log10" => Builtin::Log10, "deg_to_rad" => Builtin::DegToRad, _ => Builtin::RadToDeg,
            };
            call(b, vec![Float], Float)
        }
        "atan2" | "pow" | "snapped" => {
            arity(name, args, 2)?;
            let b = match name { "pow" => Builtin::Pow, "atan2" => Builtin::Atan2, _ => Builtin::Snapped };
            call(b, vec![Float, Float], Float)
        }
        "abs" => {
            arity(name, args, 1)?;
            if args[0] == Int { call(Builtin::AbsI, vec![Int], Int) } else { call(Builtin::AbsF, vec![Float], Float) }
        }
        "sign" => {
            arity(name, args, 1)?;
            if args[0] == Int { call(Builtin::SignI, vec![Int], Int) } else { call(Builtin::SignF, vec![Float], Float) }
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
        "wrap" => {
            arity(name, args, 3)?;
            if all_int(args) { call(Builtin::WrapI, vec![Int, Int, Int], Int) }
            else { call(Builtin::WrapF, vec![Float, Float, Float], Float) }
        }
        "lerp" | "inverse_lerp" | "smoothstep" | "move_toward" => {
            arity(name, args, 3)?;
            let b = match name {
                "lerp" => Builtin::Lerp, "inverse_lerp" => Builtin::InverseLerp,
                "smoothstep" => Builtin::Smoothstep, _ => Builtin::MoveToward,
            };
            call(b, vec![Float, Float, Float], Float)
        }
        "remap" => { arity(name, args, 5)?; call(Builtin::Remap, vec![Float; 5], Float) }
        "mod" => { arity(name, args, 2)?; call(Builtin::Mod, vec![Int, Int], Int) }
        "random" => { arity(name, args, 0)?; call(Builtin::Random, vec![], Float) }
        "random_int" => { arity(name, args, 2)?; call(Builtin::RandomInt, vec![Int, Int], Int) }
        // random
        "seed" => { arity(name, args, 1)?; call(Builtin::Seed, vec![Int], Void) }
        "randf" => { arity(name, args, 0)?; call(Builtin::RandF, vec![], Float) }
        "randi" => { arity(name, args, 0)?; call(Builtin::RandI, vec![], Int) }
        "randi_range" => { arity(name, args, 2)?; call(Builtin::RandIRange, vec![Int, Int], Int) }
        "randf_range" => { arity(name, args, 2)?; call(Builtin::RandFRange, vec![Float, Float], Float) }
        "chance" => { arity(name, args, 1)?; call(Builtin::Chance, vec![Float], Bool) }
        "rand_dir" => { arity(name, args, 0)?; call(Builtin::RandDir, vec![], Vec(2)) }
        // time
        "time" => { arity(name, args, 0)?; call(Builtin::Time, vec![], Float) }
        "ticks_msec" => { arity(name, args, 0)?; call(Builtin::TicksMsec, vec![], Int) }
        "ticks_usec" => { arity(name, args, 0)?; call(Builtin::TicksUsec, vec![], Int) }
        "now" => { arity(name, args, 0)?; call(Builtin::Now, vec![], Float) }
        "sleep" => { arity(name, args, 1)?; call(Builtin::Sleep, vec![Float], Void) }
        "date" => {
            let map = Map { cap: 8, key: std::rc::Rc::new(Str), val: std::rc::Rc::new(Int) };
            match args.len() {
                0 => call(Builtin::Date0, vec![], map),
                1 => call(Builtin::Date1, vec![Float], map),
                n => Err(format!("date() expects 0 or 1 arguments, got {n}")),
            }
        }
        "date_string" => match args.len() {
            0 => call(Builtin::DateString0, vec![], Str),
            1 => call(Builtin::DateString1, vec![Float], Str),
            n => Err(format!("date_string() expects 0 or 1 arguments, got {n}")),
        },
        // os
        "arg_count" => { arity(name, args, 0)?; call(Builtin::ArgCount, vec![], Int) }
        "arg" => { arity(name, args, 1)?; call(Builtin::Arg, vec![Int], Str) }
        "env" => { arity(name, args, 1)?; call(Builtin::Env, vec![Str], Str) }
        "has_env" => { arity(name, args, 1)?; call(Builtin::HasEnv, vec![Str], Bool) }
        "platform" => { arity(name, args, 0)?; call(Builtin::Platform, vec![], Str) }
        "exit" => { arity(name, args, 1)?; call(Builtin::Exit, vec![Int], Void) }
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
        (Str, "trim_start") => m(Builtin::StrTrimStart, vec![], Str),
        (Str, "trim_end") => m(Builtin::StrTrimEnd, vec![], Str),
        (Str, "pad_left") => m(Builtin::StrPadLeft, vec![Int, Str], Str),
        (Str, "pad_right") => m(Builtin::StrPadRight, vec![Int, Str], Str),
        (Str, "reverse") => m(Builtin::StrReverse, vec![], Str),
        (Str, "count") => m(Builtin::StrCount, vec![Str], Int),
        (Str, "rfind") => m(Builtin::StrRfind, vec![Str], Int),
        (Str, "is_int") => m(Builtin::StrIsInt, vec![], Bool),
        (Str, "is_float") => m(Builtin::StrIsFloat, vec![], Bool),
        (Str, "is_alpha") => m(Builtin::StrIsAlpha, vec![], Bool),
        (Str, "is_digit") => m(Builtin::StrIsDigit, vec![], Bool),
        (Str, "is_space") => m(Builtin::StrIsSpace, vec![], Bool),
        (Str, "code") => m(Builtin::StrCode, vec![], Int),
        (Vec(_), "length") => m(Builtin::VecLength, vec![], Float),
        (Vec(n), "normalized") => m(Builtin::VecNormalized, vec![], Vec(*n)),
        (Vec(n), "dot") => m(Builtin::VecDot, vec![Vec(*n)], Float),
        (Vec(n), "distance") => m(Builtin::VecDistance, vec![Vec(*n)], Float),
        (Vec(3), "cross") => m(Builtin::VecCross, vec![Vec(3)], Vec(3)),
        _ => return None,
    })
}
