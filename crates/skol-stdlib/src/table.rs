//! Fixed-capacity tables. Lists (`T8[i]`) and key/value tables (`T8[s, i]`)
//! share one type and one set of operations, matching the four dispatchers
//! `.iter`, `.pos`, `.edit` and `.view`.

use std::collections::HashMap;
use std::rc::Rc;

use skol_ir::Type;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key { Int(i64), Str(Rc<str>), Bool(bool) }

impl Key {
    pub fn from_value(v: &Value) -> Result<Key, String> {
        match v {
            Value::Int(i) => Ok(Key::Int(*i)),
            Value::Str(s) => Ok(Key::Str(s.clone())),
            Value::Bool(b) => Ok(Key::Bool(*b)),
            _ => Err("table keys must be i, s or bool".into()),
        }
    }
    pub fn to_value(&self) -> Value {
        match self { Key::Int(i) => Value::Int(*i), Key::Str(s) => Value::Str(s.clone()), Key::Bool(b) => Value::Bool(*b) }
    }
    fn describe(&self) -> String {
        match self { Key::Int(i) => i.to_string(), Key::Str(s) => format!("{s:?}"), Key::Bool(b) => b.to_string() }
    }
}

#[derive(Debug)]
pub enum TableData {
    List(Vec<Value>),
    /// Insertion-ordered map.
    Map { keys: Vec<Key>, vals: Vec<Value>, index: HashMap<Key, usize> },
}

#[derive(Debug)]
pub struct Table {
    pub cap: usize,
    /// `None` for list tables.
    pub key_ty: Option<Type>,
    pub val_ty: Type,
    pub data: TableData,
}

impl Drop for Table {
    fn drop(&mut self) {
        let nested = match &self.data {
            TableData::List(v) => v.iter().any(|x| !matches!(x, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) | Value::Vec2(..) | Value::Vec(_))),
            TableData::Map { vals, .. } => vals.iter().any(|x| !matches!(x, Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_) | Value::Vec2(..) | Value::Vec(_))),
        };
        if nested {
            let mut out = Vec::new();
            self.take_values(&mut out);
            crate::value::drop_values(out);
        }
    }
}

impl Table {
    /// Move every stored value out (used for iterative destruction).
    pub fn take_values(&mut self, out: &mut Vec<Value>) {
        match &mut self.data {
            TableData::List(v) => out.append(v),
            TableData::Map { vals, keys, index } => { out.append(vals); keys.clear(); index.clear(); }
        }
    }

    pub fn new_list(cap: usize, elem: Type, items: Vec<Value>) -> Table {
        Table { cap, key_ty: None, val_ty: elem, data: TableData::List(items) }
    }

    pub fn new_map(cap: usize, key: Type, val: Type) -> Table {
        Table { cap, key_ty: Some(key), val_ty: val, data: TableData::Map { keys: vec![], vals: vec![], index: HashMap::new() } }
    }

    pub fn ty(&self) -> Type {
        match &self.key_ty {
            None => Type::List { cap: self.cap, elem: Rc::new(self.val_ty.clone()) },
            Some(k) => Type::Map { cap: self.cap, key: Rc::new(k.clone()), val: Rc::new(self.val_ty.clone()) },
        }
    }

    pub fn len(&self) -> usize {
        match &self.data { TableData::List(v) => v.len(), TableData::Map { keys, .. } => keys.len() }
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    fn full_error(&self) -> String { format!("table is full (capacity {})", self.cap) }

    fn index(&self, k: &Value) -> Result<usize, String> {
        let i = match k { Value::Int(i) => *i, _ => return Err("list index must be an i".into()) };
        let len = self.len();
        if i < 0 || i as usize >= len {
            return Err(format!("index {i} is out of range for a table of length {len}"));
        }
        Ok(i as usize)
    }

    /// Insert or update a map entry; errors when inserting into a full table.
    pub fn map_insert(&mut self, key: Key, value: Value) -> Result<(), String> {
        let cap = self.cap;
        let TableData::Map { keys, vals, index } = &mut self.data else { return Err("not a key/value table".into()) };
        if let Some(&i) = index.get(&key) {
            vals[i] = value;
            return Ok(());
        }
        if keys.len() >= cap { return Err(format!("table is full (capacity {cap})")); }
        index.insert(key.clone(), keys.len());
        keys.push(key);
        vals.push(value);
        Ok(())
    }

    // ---- .iter(...)

    pub fn sum(&self) -> Value {
        let vals: &[Value] = match &self.data { TableData::List(v) => v, TableData::Map { vals, .. } => vals };
        if self.val_ty == Type::Float {
            Value::Float(vals.iter().map(|v| if let Value::Float(f) = v { *f } else { 0.0 }).sum())
        } else {
            Value::Int(vals.iter().fold(0i64, |a, v| a.wrapping_add(if let Value::Int(i) = v { *i } else { 0 })))
        }
    }

    /// New `T[i]` (same capacity) with the even elements, in order.
    pub fn even(&self) -> Table {
        let items = match &self.data {
            TableData::List(v) => v.iter().filter(|x| matches!(x, Value::Int(i) if i % 2 == 0)).cloned().collect(),
            TableData::Map { .. } => vec![],
        };
        Table::new_list(self.cap, Type::Int, items)
    }

    /// Reset the slot at `k` to `zero`. The slot (and the map key) stay, so the
    /// table length is unchanged.
    pub fn cut(&mut self, k: &Value, zero: Value) -> Result<(), String> {
        match &self.data {
            TableData::List(_) => {
                let i = self.index(k)?;
                if let TableData::List(v) = &mut self.data { v[i] = zero; }
                Ok(())
            }
            TableData::Map { index, .. } => {
                let key = Key::from_value(k)?;
                let Some(&i) = index.get(&key) else { return Err(format!("key {} not found", key.describe())) };
                if let TableData::Map { vals, .. } = &mut self.data { vals[i] = zero; }
                Ok(())
            }
        }
    }

    // ---- .pos(...)

    pub fn get(&self, k: &Value) -> Result<Value, String> {
        match &self.data {
            TableData::List(v) => Ok(v[self.index(k)?].clone()),
            TableData::Map { vals, index, .. } => {
                let key = Key::from_value(k)?;
                index.get(&key).map(|&i| vals[i].clone()).ok_or_else(|| format!("key {} not found", key.describe()))
            }
        }
    }

    pub fn set(&mut self, k: &Value, value: Value) -> Result<(), String> {
        match &self.data {
            TableData::List(_) => {
                let i = self.index(k)?;
                if let TableData::List(v) = &mut self.data { v[i] = value; }
                Ok(())
            }
            TableData::Map { .. } => self.map_insert(Key::from_value(k)?, value),
        }
    }

    pub fn has(&self, k: &Value) -> bool {
        match &self.data {
            TableData::List(v) => matches!(k, Value::Int(i) if *i >= 0 && (*i as usize) < v.len()),
            TableData::Map { index, .. } => Key::from_value(k).map(|key| index.contains_key(&key)).unwrap_or(false),
        }
    }

    // ---- .edit(...)

    pub fn push(&mut self, value: Value) -> Result<(), String> {
        let full = self.len() >= self.cap;
        let err = self.full_error();
        match &mut self.data {
            TableData::List(v) => { if full { return Err(err); } v.push(value); Ok(()) }
            TableData::Map { .. } => Err("push only works on list tables".into()),
        }
    }

    pub fn pop(&mut self) -> Result<Value, String> {
        match &mut self.data {
            TableData::List(v) => v.pop().ok_or_else(|| "can't pop from an empty table".to_string()),
            TableData::Map { .. } => Err("pop only works on list tables".into()),
        }
    }

    pub fn clear(&mut self) {
        match &mut self.data {
            TableData::List(v) => v.clear(),
            TableData::Map { keys, vals, index } => { keys.clear(); vals.clear(); index.clear(); }
        }
    }

    // ---- .view(...)

    pub fn keys(&self) -> Table {
        match &self.data {
            TableData::List(v) => Table::new_list(self.cap, Type::Int, (0..v.len() as i64).map(Value::Int).collect()),
            TableData::Map { keys, .. } => Table::new_list(
                self.cap, self.key_ty.clone().unwrap_or(Type::Int), keys.iter().map(Key::to_value).collect()),
        }
    }

    pub fn vals(&self) -> Table {
        let vals = match &self.data { TableData::List(v) => v.clone(), TableData::Map { vals, .. } => vals.clone() };
        Table::new_list(self.cap, self.val_ty.clone(), vals)
    }

    /// Element at position `i` when iterating with `for` (map: the key).
    pub fn iter_at(&self, i: usize) -> Option<Value> {
        match &self.data {
            TableData::List(v) => v.get(i).cloned(),
            TableData::Map { keys, .. } => keys.get(i).map(Key::to_value),
        }
    }
}
