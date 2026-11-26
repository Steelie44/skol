//! Fixed-capacity tables. Lists (`T8[i]`) and key/value tables (`T8[s, i]`)
//! share one type and one set of operations, matching the four dispatchers
//! `.iter`, `.pos`, `.edit` and `.view`.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use skol_ir::Type;

use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key { Int(i64), Str(Rc<str>), Bool(bool) }

impl std::hash::Hash for Key {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match self {
            Key::Int(i) => h.write_u64(*i as u64),
            Key::Str(s) => h.write(s.as_bytes()),
            Key::Bool(b) => h.write_u64(*b as u64 | 0x9e37_79b9_0000_0000),
        }
    }
}

/// A fast non-cryptographic hasher (FxHash) for table keys. Tables have a
/// fixed, small capacity, so a deliberately colliding key set can only cost
/// O(capacity) per lookup: SipHash's flooding protection isn't needed.
#[derive(Default, Clone, Copy)]
pub struct FxHasher { hash: u64 }

const FX_K: u64 = 0x517c_c1b7_2722_0a95;

impl FxHasher {
    #[inline(always)]
    fn add(&mut self, w: u64) { self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(FX_K); }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks { self.add(u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])); }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
        self.add(bytes.len() as u64);
    }
    #[inline]
    fn write_u64(&mut self, n: u64) { self.add(n); }
    #[inline]
    fn finish(&self) -> u64 { self.hash }
}

pub type KeyIndex = HashMap<Key, usize, BuildHasherDefault<FxHasher>>;

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
    pub fn describe(&self) -> String {
        match self { Key::Int(i) => i.to_string(), Key::Str(s) => format!("{s:?}"), Key::Bool(b) => b.to_string() }
    }
}

#[derive(Debug)]
pub enum TableData {
    List(Vec<Value>),
    /// Insertion-ordered map.
    Map { keys: Vec<Key>, vals: Vec<Value>, index: KeyIndex },
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
        Table { cap, key_ty: Some(key), val_ty: val, data: TableData::Map { keys: vec![], vals: vec![], index: KeyIndex::default() } }
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

    // ---- helpers shared by the extra operations

    /// The stored values (list elements or map values), in order.
    pub fn values(&self) -> &[Value] {
        match &self.data { TableData::List(v) => v, TableData::Map { vals, .. } => vals }
    }

    fn list_mut(&mut self) -> Result<&mut Vec<Value>, String> {
        match &mut self.data { TableData::List(v) => Ok(v), TableData::Map { .. } => Err("this only works on list tables".into()) }
    }

    fn rebuild_index(&mut self) {
        if let TableData::Map { keys, index, .. } = &mut self.data {
            index.clear();
            for (i, k) in keys.iter().enumerate() { index.insert(k.clone(), i); }
        }
    }

    /// A new, independent table with the same type and contents (shallow).
    pub fn copy(&self) -> Table {
        let data = match &self.data {
            TableData::List(v) => TableData::List(v.clone()),
            TableData::Map { keys, vals, index } => TableData::Map { keys: keys.clone(), vals: vals.clone(), index: index.clone() },
        };
        Table { cap: self.cap, key_ty: self.key_ty.clone(), val_ty: self.val_ty.clone(), data }
    }

    // ---- extra .iter(...)

    pub fn count(&self, v: &Value) -> i64 { self.values().iter().filter(|x| x.equals(v)).count() as i64 }

    pub fn find(&self, v: &Value) -> i64 { self.values().iter().position(|x| x.equals(v)).map(|i| i as i64).unwrap_or(-1) }

    pub fn contains(&self, v: &Value) -> bool { self.values().iter().any(|x| x.equals(v)) }

    /// Smallest (or largest) value; errors on an empty table.
    pub fn extreme(&self, largest: bool) -> Result<Value, String> {
        let vals = self.values();
        let mut best = vals.first().ok_or_else(|| format!("can't take the {} of an empty table", if largest { "max" } else { "min" }))?;
        for v in &vals[1..] {
            let better = match cmp_values(v, best) {
                Some(std::cmp::Ordering::Greater) => largest,
                Some(std::cmp::Ordering::Less) => !largest,
                _ => false,
            };
            if better { best = v; }
        }
        Ok(best.clone())
    }

    // ---- extra .pos(...)

    pub fn first(&self) -> Result<Value, String> {
        self.values().first().cloned().ok_or_else(|| "the table is empty".to_string())
    }

    pub fn last(&self) -> Result<Value, String> {
        self.values().last().cloned().ok_or_else(|| "the table is empty".to_string())
    }

    pub fn get_or(&self, k: &Value, default: Value) -> Value {
        match &self.data {
            TableData::List(v) => match k { Value::Int(i) if *i >= 0 && (*i as usize) < v.len() => v[*i as usize].clone(), _ => default },
            TableData::Map { vals, index, .. } => match Key::from_value(k).ok().and_then(|key| index.get(&key).copied()) {
                Some(i) => vals[i].clone(),
                None => default,
            },
        }
    }

    // ---- extra .edit(...)

    /// Insert before position `i` (0..=len), shifting later elements up.
    pub fn insert(&mut self, i: &Value, value: Value) -> Result<(), String> {
        let cap = self.cap;
        let v = self.list_mut()?;
        let Value::Int(i) = *i else { return Err("list index must be an i".into()) };
        if i < 0 || i as usize > v.len() {
            return Err(format!("insert position {i} is out of range for a table of length {}", v.len()));
        }
        if v.len() >= cap { return Err(format!("table is full (capacity {cap})")); }
        v.insert(i as usize, value);
        Ok(())
    }

    /// Remove an element (shifting later ones down) or a map entry; returns the value.
    pub fn remove(&mut self, k: &Value) -> Result<Value, String> {
        match &mut self.data {
            TableData::List(v) => {
                let len = v.len();
                match k {
                    Value::Int(i) if *i >= 0 && (*i as usize) < len => Ok(v.remove(*i as usize)),
                    Value::Int(i) => Err(format!("index {i} is out of range for a table of length {len}")),
                    _ => Err("list index must be an i".into()),
                }
            }
            TableData::Map { keys, vals, index } => {
                let key = Key::from_value(k)?;
                let Some(i) = index.remove(&key) else { return Err(format!("key {} not found", key.describe())) };
                keys.remove(i);
                let v = vals.remove(i);
                // Entries after the removed one moved down by one.
                for j in i..keys.len() { index.insert(keys[j].clone(), j); }
                Ok(v)
            }
        }
    }

    /// Remove element `i` by moving the last element into its place (O(1),
    /// changes order).
    pub fn swap_remove(&mut self, i: &Value) -> Result<Value, String> {
        let v = self.list_mut()?;
        let len = v.len();
        match i {
            Value::Int(i) if *i >= 0 && (*i as usize) < len => Ok(v.swap_remove(*i as usize)),
            Value::Int(i) => Err(format!("index {i} is out of range for a table of length {len}")),
            _ => Err("list index must be an i".into()),
        }
    }

    pub fn pop_front(&mut self) -> Result<Value, String> {
        let v = self.list_mut()?;
        if v.is_empty() { return Err("can't pop from an empty table".into()); }
        Ok(v.remove(0))
    }

    /// Append another table's elements (list) or merge its entries (map).
    pub fn extend(&mut self, other: &Table) -> Result<(), String> {
        match &other.data {
            TableData::List(items) => {
                let cap = self.cap;
                let v = self.list_mut()?;
                if v.len() + items.len() > cap {
                    return Err(format!("extend would hold {} elements but the table holds at most {cap}", v.len() + items.len()));
                }
                v.extend(items.iter().cloned());
                Ok(())
            }
            TableData::Map { keys, vals, .. } => {
                for (k, v) in keys.iter().zip(vals.iter()) { self.map_insert(k.clone(), v.clone())?; }
                Ok(())
            }
        }
    }

    /// Sort a list ascending (stable), or a map's entries by key.
    pub fn sort(&mut self) {
        match &mut self.data {
            TableData::List(v) => v.sort_by(|a, b| cmp_values(a, b).unwrap_or(std::cmp::Ordering::Equal)),
            TableData::Map { keys, vals, .. } => {
                let mut pairs: Vec<(Key, Value)> = keys.drain(..).zip(vals.drain(..)).collect();
                pairs.sort_by(|a, b| cmp_values(&a.0.to_value(), &b.0.to_value()).unwrap_or(std::cmp::Ordering::Equal));
                for (k, v) in pairs { keys.push(k); vals.push(v); }
            }
        }
        self.rebuild_index();
    }

    pub fn reverse(&mut self) {
        match &mut self.data {
            TableData::List(v) => v.reverse(),
            TableData::Map { keys, vals, .. } => { keys.reverse(); vals.reverse(); }
        }
        self.rebuild_index();
    }

    /// Replace the list's elements (same table, so other references see it).
    pub fn replace_list(&mut self, items: Vec<Value>) -> Result<(), String> {
        let v = self.list_mut()?;
        *v = items;
        Ok(())
    }

    // ---- extra .view(...)

    /// Elements `start..end` as a new table of the same type.
    pub fn slice(&self, start: &Value, end: &Value) -> Result<Table, String> {
        let (Value::Int(a), Value::Int(b)) = (start, end) else { return Err("slice bounds must be i".into()) };
        let (a, b) = (*a, *b);
        let len = self.len() as i64;
        if a < 0 || b < a || b > len {
            return Err(format!("slice({a}, {b}) is out of range for a table of length {len}"));
        }
        Ok(Table::new_list(self.cap, self.val_ty.clone(), self.values()[a as usize..b as usize].to_vec()))
    }

    /// Element at position `i` when iterating with `for` (map: the key).
    pub fn iter_at(&self, i: usize) -> Option<Value> {
        match &self.data {
            TableData::List(v) => v.get(i).cloned(),
            TableData::Map { keys, .. } => keys.get(i).map(Key::to_value),
        }
    }
}

/// Ordering for sortable values (i, f, s, bool); `None` for anything else.
pub fn cmp_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => Some(x.total_cmp(y)),
        (Value::Int(x), Value::Float(y)) => Some((*x as f64).total_cmp(y)),
        (Value::Float(x), Value::Int(y)) => Some(x.total_cmp(&(*y as f64))),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}
