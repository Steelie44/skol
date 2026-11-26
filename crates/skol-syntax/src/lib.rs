//! Skol front end: lexer, parser and untyped AST.

#![forbid(unsafe_code)]

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::Span;

/// A lexing or parsing failure with a 1-based source position.
#[derive(Debug, Clone, PartialEq)]
pub struct SyntaxError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Tokenize and parse a whole source file.
pub fn parse(source: &str) -> Result<ast::Program, SyntaxError> {
    let tokens = lexer::Lexer::new(source).tokenize().map_err(to_syntax_error)?;
    parser::Parser::new(tokens).parse_program().map_err(to_syntax_error)
}

pub fn tokenize(source: &str) -> Result<Vec<lexer::Token>, SyntaxError> {
    lexer::Lexer::new(source).tokenize().map_err(to_syntax_error)
}

fn to_syntax_error(message: String) -> SyntaxError {
    let (line, col) = find_position(&message).unwrap_or((1, 1));
    SyntaxError { message, line, col }
}

/// Convert a byte offset into a 1-based (line, column) pair.
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset { break; }
        if ch == '\n' { line += 1; col = 1; } else { col += 1; }
    }
    (line, col)
}

/// The lexer and parser embed positions as "line:col" in their messages.
fn find_position(msg: &str) -> Option<(usize, usize)> {
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let s1 = i;
            while i < b.len() && b[i].is_ascii_digit() { i += 1; }
            if i + 1 < b.len() && b[i] == b':' && b[i + 1].is_ascii_digit() {
                let line: usize = msg[s1..i].parse().ok()?;
                let s2 = i + 1;
                let mut j = s2;
                while j < b.len() && b[j].is_ascii_digit() { j += 1; }
                let col: usize = msg[s2..j].parse().ok()?;
                return Some((line, col));
            }
        } else {
            i += 1;
        }
    }
    None
}
