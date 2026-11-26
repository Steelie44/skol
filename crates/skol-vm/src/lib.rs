//! Skol bytecode compiler (`bytecode.rs`) and register virtual machine (`vm.rs`).

#![forbid(unsafe_code)]

pub mod bytecode;
pub mod vm;

pub use bytecode::{compile, compile_into, Bytecode, Op};
pub use vm::{HostFn, Limits, RuntimeError, TraceEntry, Vm};
