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

/// Mix a user seed so nearby seeds give unrelated sequences (splitmix64).
pub fn mix_seed(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

fn unit_float(state: &mut u64) -> f64 { (next_random(state) >> 11) as f64 / (1u64 << 53) as f64 }

fn rand_range(state: &mut u64, lo: i64, hi: i64) -> i64 {
    let span = (hi as i128 - lo as i128 + 1) as u128;
    (lo as i128 + (next_random(state) as u128 % span) as i128) as i64
}

/// Exported for table `.edit(shuffle)` / `.view(pick)` in the VM.
pub fn random_index(state: &mut u64, len: usize) -> usize { rand_range(state, 0, len as i64 - 1) as usize }

fn check_size(n: usize, what: &str) -> Result<(), String> {
    if n > crate::MAX_STRING_BYTES { Err(format!("{what} would build a string of {n} bytes")) } else { Ok(()) }
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day, hour, minute, second, weekday 0=Sunday, day of year 1..)
pub fn date_parts(unix: f64) -> Result<[i64; 8], String> {
    if !unix.is_finite() || unix.abs() > 1e13 { return Err(format!("{} is not a valid time", crate::format::format_float(unix))); }
    let secs = unix.floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let jan1 = days_from_civil(y, 1, 1);
    Ok([y, m, d, rem / 3600, rem % 3600 / 60, rem % 60, (days + 4).rem_euclid(7), days - jan1 + 1])
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn unix_now() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

fn date_table(unix: f64) -> Result<Value, String> {
    let p = date_parts(unix)?;
    let mut t = crate::table::Table::new_map(8, skol_ir::Type::Str, skol_ir::Type::Int);
    for (name, v) in ["year", "month", "day", "hour", "minute", "second", "weekday", "yearday"].iter().zip(p) {
        t.map_insert(crate::table::Key::Str((*name).into()), Value::Int(v))?;
    }
    Ok(Value::Table(std::rc::Rc::new(std::cell::RefCell::new(t))))
}

fn date_string(unix: f64) -> Result<Value, String> {
    let p = date_parts(unix)?;
    Ok(Value::str(&format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", p[0], p[1], p[2], p[3], p[4], p[5])))
}

fn string_list(items: Vec<Value>, cap: i64, what: &str) -> Result<Value, String> {
    if items.len() as i64 > cap {
        return Err(format!("{what} produced {} entries but the table holds at most {cap}", items.len()));
    }
    Ok(Value::Table(std::rc::Rc::new(std::cell::RefCell::new(
        crate::table::Table::new_list(cap.max(0) as usize, skol_ir::Type::Str, items)))))
}

/// `fmt("{} has {.1} hp", name, hp)`.
pub fn format_template(template: &str, args: &[Value], host: &dyn Host) -> Result<String, String> {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0usize;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => { chars.next(); out.push('{'); }
            '}' if chars.peek() == Some(&'}') => { chars.next(); out.push('}'); }
            '{' => {
                let mut spec = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) if spec.len() < 8 => spec.push(ch),
                        _ => return Err("fmt: unclosed '{' in template (write {{ for a literal brace)".into()),
                    }
                }
                let Some(arg) = args.get(next) else {
                    return Err(format!("fmt: the template has more placeholders than the {} value{} given", args.len(), if args.len() == 1 { "" } else { "s" }));
                };
                next += 1;
                if spec.is_empty() {
                    out.push_str(&to_display(arg, host));
                } else if let Some(digits) = spec.strip_prefix('.').and_then(|d| d.parse::<usize>().ok()).filter(|d| *d <= 17) {
                    match arg {
                        Value::Float(x) => out.push_str(&format!("{x:.digits$}")),
                        Value::Int(i) => out.push_str(&format!("{:.digits$}", *i as f64)),
                        _ => return Err(format!("fmt: {{.{digits}}} needs a number, got {}", type_of(arg, host))),
                    }
                } else {
                    return Err(format!("fmt: unknown placeholder {{{spec}}}; use {{}} or {{.2}}"));
                }
            }
            '}' => return Err("fmt: unmatched '}' in template (write }} for a literal brace)".into()),
            c => out.push(c),
        }
        if out.len() > crate::MAX_STRING_BYTES { return Err("fmt would build a string that is too long".into()); }
    }
    if next < args.len() {
        return Err(format!("fmt: {} value{} given but the template has only {next} placeholder{}",
            args.len(), if args.len() == 1 { " was" } else { "s were" }, if next == 1 { "" } else { "s" }));
    }
    Ok(out)
}

fn pad(text: &str, n: i64, fill: &str, left: bool) -> Result<Value, String> {
    let mut fc = fill.chars();
    let (Some(ch), None) = (fc.next(), fc.next()) else { return Err("pad fill must be exactly one character".into()) };
    let len = text.chars().count() as i64;
    if n <= len { return Ok(Value::str(text)); }
    let extra = (n - len) as usize;
    check_size(text.len().saturating_add(extra.saturating_mul(ch.len_utf8())), "pad")?;
    let padding: String = std::iter::repeat(ch).take(extra).collect();
    Ok(Value::str(&if left { padding + text } else { text.to_string() + &padding }))
}

fn fs_err(what: &str, path: &str, e: std::io::Error) -> String { format!("couldn't {what} {path:?}: {e}") }

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
        ReadFile | WriteFile | AppendFile | FileExists | ReadLines | ListDir | DeleteFile | DeleteDir | MakeDir
        | DirExists | RenameFile if !host.fs_allowed() => {
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
        DirExists => Value::Bool(std::path::Path::new(s0()?).is_dir()),
        ReadLines => {
            let path = s0()?;
            if let Ok(meta) = std::fs::metadata(path) {
                if meta.len() > crate::MAX_STRING_BYTES as u64 {
                    return Err(format!("{path:?} is too large to read ({} bytes)", meta.len()));
                }
            }
            let text = std::fs::read_to_string(path).map_err(|e| fs_err("read", path, e))?;
            string_list(text.lines().map(Value::str).collect(), i1()?, &format!("read_lines({path:?})"))?
        }
        ListDir => {
            let path = s0()?;
            let mut names: Vec<String> = Vec::new();
            for entry in std::fs::read_dir(path).map_err(|e| fs_err("list", path, e))? {
                let entry = entry.map_err(|e| fs_err("list", path, e))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
                if names.len() as i64 > i1()? { break; }
            }
            names.sort();
            string_list(names.iter().map(|n| Value::str(n)).collect(), i1()?, &format!("list_dir({path:?})"))?
        }
        DeleteFile => { let p = s0()?; std::fs::remove_file(p).map_err(|e| fs_err("delete", p, e))?; Value::Void }
        DeleteDir => { let p = s0()?; std::fs::remove_dir(p).map_err(|e| fs_err("delete folder", p, e))?; Value::Void }
        MakeDir => { let p = s0()?; std::fs::create_dir_all(p).map_err(|e| fs_err("create folder", p, e))?; Value::Void }
        RenameFile => {
            let (from, to) = (s0()?, s1()?);
            std::fs::rename(from, to).map_err(|e| format!("couldn't rename {from:?} to {to:?}: {e}"))?;
            Value::Void
        }
        Fmt => Value::str(&format_template(s0()?, &args[1..], &*host)?),
        Chr => {
            let code = i0()?;
            let c = u32::try_from(code).ok().and_then(char::from_u32).ok_or_else(|| format!("{code} is not a valid character code"))?;
            Value::str(c.encode_utf8(&mut [0u8; 4]))
        }
        Range1 => Value::range(0, i0()?),
        Range2 => Value::range(i0()?, i1()?),

        Sqrt => Value::Float(f0()?.sqrt()),
        Sin => Value::Float(f0()?.sin()),
        Cos => Value::Float(f0()?.cos()),
        Tan => Value::Float(f0()?.tan()),
        Asin => Value::Float(f0()?.asin()),
        Acos => Value::Float(f0()?.acos()),
        Atan => Value::Float(f0()?.atan()),
        Exp => Value::Float(f0()?.exp()),
        Log => Value::Float(f0()?.ln()),
        Log10 => Value::Float(f0()?.log10()),
        DegToRad => Value::Float(f0()?.to_radians()),
        RadToDeg => Value::Float(f0()?.to_degrees()),
        SignI => Value::Int(i0()?.signum()),
        SignF => { let x = f0()?; Value::Float(if x == 0.0 || x.is_nan() { x } else { x.signum() }) }
        WrapI => {
            let (x, lo, hi) = (i0()?, i1()?, i2()?);
            if hi <= lo { return Err("wrap: max must be greater than min".into()); }
            let span = hi as i128 - lo as i128;
            Value::Int((lo as i128 + (x as i128 - lo as i128).rem_euclid(span)) as i64)
        }
        WrapF => {
            let (x, lo, hi) = (f0()?, f1()?, f2()?);
            if !(hi > lo) { return Err("wrap: max must be greater than min".into()); }
            Value::Float(lo + (x - lo).rem_euclid(hi - lo))
        }
        MoveToward => {
            let (from, to, delta) = (f0()?, f1()?, f2()?);
            Value::Float(if (to - from).abs() <= delta { to } else { from + (to - from).signum() * delta })
        }
        Smoothstep => {
            let (e0, e1, x) = (f0()?, f1()?, f2()?);
            if e0 == e1 { Value::Float(if x < e0 { 0.0 } else { 1.0 }) } else {
                let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                Value::Float(t * t * (3.0 - 2.0 * t))
            }
        }
        InverseLerp => { let (a, c, v) = (f0()?, f1()?, f2()?); Value::Float(if a == c { 0.0 } else { (v - a) / (c - a) }) }
        Remap => {
            let v: Vec<f64> = args.iter().map(|x| f(x).ok_or_else(|| bad(b))).collect::<Result<_, _>>()?;
            let t = if v[1] == v[2] { 0.0 } else { (v[0] - v[1]) / (v[2] - v[1]) };
            Value::Float(v[3] + (v[4] - v[3]) * t)
        }
        Snapped => { let (x, step) = (f0()?, f1()?); Value::Float(if step == 0.0 { x } else { (x / step).round() * step }) }
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
        Random | RandF => Value::Float(unit_float(host.rng_state())),
        RandomInt | RandIRange => {
            let (lo, hi) = (i0()?, i1()?);
            if lo > hi { return Err(format!("{}: min is greater than max", if b == RandomInt { "random_int" } else { "randi_range" })); }
            Value::Int(rand_range(host.rng_state(), lo, hi))
        }
        Seed => { *host.rng_state() = mix_seed(i0()? as u64); Value::Void }
        RandI => Value::Int((next_random(host.rng_state()) >> 1) as i64),
        RandFRange => {
            let (lo, hi) = (f0()?, f1()?);
            if lo > hi { return Err("randf_range: min is greater than max".into()); }
            Value::Float(lo + (hi - lo) * unit_float(host.rng_state()))
        }
        Chance => { let p = f0()?; Value::Bool(unit_float(host.rng_state()) < p) }
        RandDir => {
            let a = unit_float(host.rng_state()) * std::f64::consts::TAU;
            Value::Vec2(a.cos(), a.sin())
        }
        Time => Value::Float(host.elapsed_secs()),
        TicksMsec => Value::Int((host.elapsed_secs() * 1000.0) as i64),
        TicksUsec => Value::Int((host.elapsed_secs() * 1_000_000.0) as i64),
        Now => Value::Float(unix_now()),
        Sleep => {
            if !host.os_allowed() { return Err("sleep is disabled for this script".into()); }
            let secs = f0()?;
            if !secs.is_finite() || secs < 0.0 { return Err("sleep needs a non-negative number of seconds".into()); }
            if secs > 86_400.0 { return Err("sleep is limited to one day".into()); }
            let _ = std::io::Write::flush(&mut std::io::stdout());
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            Value::Void
        }
        Date0 => date_table(unix_now())?,
        Date1 => date_table(f0()?)?,
        DateString0 => date_string(unix_now())?,
        DateString1 => date_string(f0()?)?,
        ArgCount => Value::Int(host.script_args().len() as i64),
        Arg => {
            let i = i0()?;
            let args = host.script_args();
            match usize::try_from(i).ok().and_then(|i| args.get(i)) {
                Some(a) => Value::str(a),
                None => return Err(format!("arg({i}) is out of range: the script got {} argument{}", args.len(), if args.len() == 1 { "" } else { "s" })),
            }
        }
        Platform => Value::str(match std::env::consts::OS { "macos" => "macos", "windows" => "windows", "linux" => "linux", other => other }),
        Env | HasEnv | Exit if !host.os_allowed() => return Err(format!("{} is disabled for this script", match b { Env => "env", HasEnv => "has_env", _ => "exit" })),
        Env => Value::str(&std::env::var(s0()?).unwrap_or_default()),
        HasEnv => Value::Bool(std::env::var_os(s0()?).is_some()),
        Exit => {
            let code = i0()?;
            host.request_exit(code.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            return Err(format!("exit({code})"));
        }

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

        StrTrimStart => Value::str(s0()?.trim_start()),
        StrTrimEnd => Value::str(s0()?.trim_end()),
        StrPadLeft => pad(s0()?, i1()?, s2()?, true)?,
        StrPadRight => pad(s0()?, i1()?, s2()?, false)?,
        StrReverse => Value::str(&s0()?.chars().rev().collect::<String>()),
        StrCount => {
            let (hay, needle) = (s0()?, s1()?);
            if needle.is_empty() { return Err("count needs a non-empty string to look for".into()); }
            Value::Int(hay.matches(needle).count() as i64)
        }
        StrRfind => {
            let (hay, needle) = (s0()?, s1()?);
            Value::Int(hay.rfind(needle).map(|byte| hay[..byte].chars().count() as i64).unwrap_or(-1))
        }
        StrIsInt => Value::Bool(s0()?.trim().parse::<i64>().is_ok()),
        StrIsFloat => Value::Bool(s0()?.trim().parse::<f64>().is_ok()),
        StrIsAlpha => { let t = s0()?; Value::Bool(!t.is_empty() && t.chars().all(char::is_alphabetic)) }
        StrIsDigit => { let t = s0()?; Value::Bool(!t.is_empty() && t.chars().all(|c| c.is_ascii_digit())) }
        StrIsSpace => { let t = s0()?; Value::Bool(!t.is_empty() && t.chars().all(char::is_whitespace)) }
        StrCode => match s0()?.chars().next() {
            Some(c) => Value::Int(c as i64),
            None => return Err("code() of an empty string".into()),
        },
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
