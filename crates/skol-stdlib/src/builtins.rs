//! Implementations of built-in functions. Argument types were already checked
//! by skol-sema, so a mismatch here means a compiler bug.

use skol_ir::Builtin;

use crate::format::{to_display, type_of};
use crate::value::{Value, VecN};
use crate::Host;

fn bad(b: Builtin) -> String { format!("internal error: bad arguments to built-in {b:?}") }

fn f(v: &Value) -> Option<f64> { if let Value::Float(x) = v { Some(*x) } else { None } }
fn i(v: &Value) -> Option<i64> { if let Value::Int(x) = v { Some(*x) } else { None } }
fn s(v: &Value) -> Option<&str> { if let Value::Str(x) = v { Some(x) } else { None } }
fn vn(v: &Value) -> Option<VecN> { v.as_vec() }

fn next_random(state: &mut u64) -> u64 {
    // xorshift64*
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn float_to_int(x: f64) -> Result<Value, String> {
    if !x.is_finite() || x >= 9.2e18 || x <= -9.2e18 {
        return Err(format!("{} can't be converted to an i", crate::format::format_float(x)));
    }
    Ok(Value::Int(x as i64))
}

pub fn call(b: Builtin, args: &[Value], host: &mut dyn Host) -> Result<Value, String> {
    use Builtin::*;
    let a0 = args.first();
    let a1 = args.get(1);
    let a2 = args.get(2);
    let f0 = || a0.and_then(f).ok_or_else(|| bad(b));
    let f1 = || a1.and_then(f).ok_or_else(|| bad(b));
    let f2 = || a2.and_then(f).ok_or_else(|| bad(b));
    let i0 = || a0.and_then(i).ok_or_else(|| bad(b));
    let i1 = || a1.and_then(i).ok_or_else(|| bad(b));
    let i2 = || a2.and_then(i).ok_or_else(|| bad(b));
    let s0 = || a0.and_then(s).ok_or_else(|| bad(b));
    let s1 = || a1.and_then(s).ok_or_else(|| bad(b));
    let s2 = || a2.and_then(s).ok_or_else(|| bad(b));
    let v0 = || a0.and_then(vn).ok_or_else(|| bad(b));
    let v1 = || a1.and_then(vn).ok_or_else(|| bad(b));

    Ok(match b {
        Print => {
            let text: Vec<String> = args.iter().map(|a| to_display(a, &*host)).collect();
            let mut line = text.join(" ");
            line.push('\n');
            host.write_out(&line);
            Value::Void
        }
        ToStr => Value::str(&to_display(a0.ok_or_else(|| bad(b))?, &*host)),
        TypeOf => Value::str(&type_of(a0.ok_or_else(|| bad(b))?, &*host)),
        Assert => {
            if !matches!(a0, Some(Value::Bool(true))) { return Err("assertion failed".into()); }
            Value::Void
        }
        AssertMsg => {
            if !matches!(a0, Some(Value::Bool(true))) { return Err(format!("assertion failed: {}", s1()?)); }
            Value::Void
        }
        Error => return Err(s0()?.to_string()),
        FloatToInt => float_to_int(f0()?)?,
        StrToInt => {
            let t = s0()?;
            Value::Int(t.trim().parse::<i64>().map_err(|_| format!("can't convert {t:?} to an i"))?)
        }
        IntToFloat => Value::Float(i0()? as f64),
        StrToFloat => {
            let t = s0()?;
            Value::Float(t.trim().parse::<f64>().map_err(|_| format!("can't convert {t:?} to an f"))?)
        }
        Input | InputPrompt => {
            if b == InputPrompt { host.write_out(s0()?); }
            Value::str(&host.read_line()?.unwrap_or_default())
        }
        ReadFile | WriteFile | AppendFile | FileExists if !host.fs_allowed() => {
            return Err("file access is disabled for this script".into());
        }
        ReadFile => {
            let path = s0()?;
            if let Ok(meta) = std::fs::metadata(path) {
                if meta.len() > crate::MAX_STRING_BYTES as u64 {
                    return Err(format!("{path:?} is too large to read ({} bytes)", meta.len()));
                }
            }
            Value::str(&std::fs::read_to_string(path).map_err(|e| format!("couldn't read {path:?}: {e}"))?)
        }
        WriteFile => {
            let path = s0()?;
            std::fs::write(path, s1()?).map_err(|e| format!("couldn't write {path:?}: {e}"))?;
            Value::Void
        }
        AppendFile => {
            use std::io::Write;
            let path = s0()?;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)
                .map_err(|e| format!("couldn't open {path:?}: {e}"))?;
            f.write_all(s1()?.as_bytes()).map_err(|e| format!("couldn't write {path:?}: {e}"))?;
            Value::Void
        }
        FileExists => Value::Bool(std::path::Path::new(s0()?).is_file()),
        Range1 => Value::range(0, i0()?),
        Range2 => Value::range(i0()?, i1()?),

        Sqrt => Value::Float(f0()?.sqrt()),
        Sin => Value::Float(f0()?.sin()),
        Cos => Value::Float(f0()?.cos()),
        Tan => Value::Float(f0()?.tan()),
        Atan2 => Value::Float(f0()?.atan2(f1()?)),
        Pow => Value::Float(f0()?.powf(f1()?)),
        AbsF => Value::Float(f0()?.abs()),
        AbsI => Value::Int(i0()?.checked_abs().ok_or("integer overflow in abs")?),
        Floor => float_to_int(f0()?.floor())?,
        Ceil => float_to_int(f0()?.ceil())?,
        Round => float_to_int(f0()?.round())?,
        MinF => Value::Float(f0()?.min(f1()?)),
        MaxF => Value::Float(f0()?.max(f1()?)),
        MinI => Value::Int(i0()?.min(i1()?)),
        MaxI => Value::Int(i0()?.max(i1()?)),
        ClampF => {
            let (x, lo, hi) = (f0()?, f1()?, f2()?);
            if lo > hi { return Err("clamp: min is greater than max".into()); }
            Value::Float(x.clamp(lo, hi))
        }
        ClampI => {
            let (x, lo, hi) = (i0()?, i1()?, i2()?);
            if lo > hi { return Err("clamp: min is greater than max".into()); }
            Value::Int(x.clamp(lo, hi))
        }
        Lerp => { let (a, c, t) = (f0()?, f1()?, f2()?); Value::Float(a + (c - a) * t) }
        Mod => {
            let (a, m) = (i0()?, i1()?);
            if m == 0 { return Err("mod by zero".into()); }
            Value::Int(a.checked_rem_euclid(m).ok_or("integer overflow in mod")?)
        }
        Random => Value::Float((next_random(host.rng_state()) >> 11) as f64 / (1u64 << 53) as f64),
        RandomInt => {
            let (lo, hi) = (i0()?, i1()?);
            if lo > hi { return Err("random_int: min is greater than max".into()); }
            let span = (hi as i128 - lo as i128 + 1) as u128;
            Value::Int((lo as i128 + (next_random(host.rng_state()) as u128 % span) as i128) as i64)
        }
        Time => Value::Float(host.elapsed_secs()),

        StrLen => Value::Int(s0()?.chars().count() as i64),
        StrUpper => Value::str(&s0()?.to_uppercase()),
        StrLower => Value::str(&s0()?.to_lowercase()),
        StrTrim => Value::str(s0()?.trim()),
        StrContains => Value::Bool(s0()?.contains(s1()?)),
        StrStartsWith => Value::Bool(s0()?.starts_with(s1()?)),
        StrEndsWith => Value::Bool(s0()?.ends_with(s1()?)),
        StrReplace => {
            let (text, from, to) = (s0()?, s1()?, s2()?);
            let hits = if from.is_empty() { text.chars().count() + 1 } else { text.matches(from).count() };
            let size = text.len().saturating_add(hits.saturating_mul(to.len()));
            if size > crate::MAX_STRING_BYTES { return Err(format!("replace would build a string of {size} bytes")); }
            Value::str(&text.replace(from, to))
        }
        StrFind => {
            let (hay, needle) = (s0()?, s1()?);
            Value::Int(hay.find(needle).map(|byte| hay[..byte].chars().count() as i64).unwrap_or(-1))
        }
        StrSubstr => {
            let (text, start, end) = (s0()?, i1()?, i2()?);
            let n = text.chars().count() as i64;
            if start < 0 || end < start || end > n {
                return Err(format!("substr({start}, {end}) is out of range for a string of length {n}"));
            }
            Value::str(&text.chars().skip(start as usize).take((end - start) as usize).collect::<String>())
        }
        StrRepeat => {
            let n = i1()?;
            if n < 0 { return Err("repeat count can't be negative".into()); }
            let text = s0()?;
            let size = (text.len() as u128) * (n as u128);
            if size > crate::MAX_STRING_BYTES as u128 { return Err(format!("repeat would build a string of {size} bytes")); }
            Value::str(&text.repeat(n as usize))
        }

        StrAt => {
            let (text, idx) = (s0()?, i1()?);
            let n = text.chars().count() as i64;
            if idx < 0 || idx >= n {
                return Err(format!("index {idx} is out of range for a string of length {n}"));
            }
            let c = text.chars().nth(idx as usize).unwrap();
            Value::str(c.encode_utf8(&mut [0u8; 4]))
        }
        StrSplit => {
            let (text, sep, cap) = (s0()?, s1()?, i2()?);
            if sep.is_empty() { return Err("split separator can't be empty".into()); }
            let parts: Vec<Value> = text.split(sep).map(Value::str).collect();
            if parts.len() as i64 > cap {
                return Err(format!("split produced {} pieces but the table holds at most {cap}", parts.len()));
            }
            Value::Table(std::rc::Rc::new(std::cell::RefCell::new(
                crate::table::Table::new_list(cap as usize, skol_ir::Type::Str, parts))))
        }
        Host(id) => host.call_host(id, args)?,
        VecLength => Value::Float(v0()?.length()),
        VecNormalized => {
            let v = v0()?;
            let len = v.length();
            Value::vec(if len == 0.0 { v } else { v.map(|c| c / len) })
        }
        VecDot => Value::Float(v0()?.dot(v1()?)),
        VecDistance => Value::Float(v0()?.map2(v1()?, |a, c| a - c).length()),
        VecCross => {
            let (a, c) = (v0()?, v1()?);
            let mut r = VecN::new(3);
            r.c[0] = a.c[1] * c.c[2] - a.c[2] * c.c[1];
            r.c[1] = a.c[2] * c.c[0] - a.c[0] * c.c[2];
            r.c[2] = a.c[0] * c.c[1] - a.c[1] * c.c[0];
            Value::vec(r)
        }
    })
}
