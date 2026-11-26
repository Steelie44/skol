//! Converting values to text for `print`, `str` and `type_of`.

use skol_ir::{DisplayType, Type};

use crate::table::TableData;
use crate::value::Value;
use crate::Host;

pub fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 { format!("{f:.1}") } else { format!("{f}") }
}

/// `print`/`str` form: strings are written raw at the top level and quoted
/// inside tables and instances.
pub fn to_display(v: &Value, host: &dyn Host) -> String {
    let mut out = String::new();
    write_value(&mut out, v, host, false, 0);
    out
}

fn write_value(out: &mut String, v: &Value, host: &dyn Host, quote: bool, depth: usize) {
    if depth > 16 { out.push_str("..."); return; }
    match v {
        Value::Void => out.push_str("void"),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&format_float(*f)),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Str(s) => if quote { out.push_str(&format!("{:?}", &**s)) } else { out.push_str(s) },
        Value::Vec2(..) | Value::Vec(_) => {
            let vn = v.as_vec().unwrap_or(crate::value::VecN::new(2));
            out.push_str(&format!("vec{}(", vn.n));
            for (i, c) in vn.comps().iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push_str(&format_float(*c));
            }
            out.push(')');
        }
        Value::Table(t) => {
            let t = t.borrow();
            out.push('{');
            match &t.data {
                TableData::List(items) => for (i, x) in items.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    write_value(out, x, host, true, depth + 1);
                },
                TableData::Map { keys, vals, .. } => for (i, (k, x)) in keys.iter().zip(vals).enumerate() {
                    if i > 0 { out.push_str(", "); }
                    write_value(out, &k.to_value(), host, true, depth + 1);
                    out.push_str(": ");
                    write_value(out, x, host, true, depth + 1);
                },
            }
            out.push('}');
        }
        Value::Instance(inst) => {
            let inst = inst.borrow();
            out.push_str(host.class_name(inst.class));
            out.push('{');
            for (i, (name, x)) in host.field_names(inst.class).iter().zip(&inst.fields).enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push_str(name);
                out.push_str(": ");
                write_value(out, x, host, true, depth + 1);
            }
            out.push('}');
        }
        Value::Func(c) => out.push_str(&format!("<fn {}>", host.func_name(c.func))),
        Value::Signal(s) => out.push_str(&format!("<signal {}>", host.signal_name(s.id))),
        Value::Range(r) => out.push_str(&format!("range({}, {})", r.0, r.1)),
        Value::Cell(c) => write_value(out, &c.borrow(), host, quote, depth),
    }
}

/// Runtime type name, matching how types are written in source.
pub fn type_of(v: &Value, host: &dyn Host) -> String {
    match v {
        Value::Void => "void".into(),
        Value::Int(_) => "i".into(),
        Value::Float(_) => "f".into(),
        Value::Bool(_) => "bool".into(),
        Value::Str(_) => "s".into(),
        Value::Vec2(..) => "vec2".into(),
        Value::Vec(vn) => format!("vec{}", vn.n),
        Value::Table(t) => DisplayType(&t.borrow().ty(), host).to_string(),
        Value::Instance(i) => host.class_name(i.borrow().class).to_string(),
        Value::Func(_) => "fn".into(),
        Value::Signal(s) => DisplayType(&Type::Signal(s.id), host).to_string(),
        Value::Range(..) => "range".into(),
        Value::Cell(c) => type_of(&c.borrow(), host),
    }
}
