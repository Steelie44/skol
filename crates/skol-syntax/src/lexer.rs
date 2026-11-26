use crate::ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Const, Var, Fn, Class, Comp, Signal, From, Use,
    If, Else, For, In, While, Return, Pass, Break, Continue,
    True, False, And, Or, Not, Lambda,

    // Built-in types
    TypeInt, TypeFloat, TypeStr, TypeBool, TypeVoid,
    TypeVec2, TypeVec3, TypeVec4, TypeTable,
    // T followed by its mandatory fixed capacity, e.g. T32 or T128.
    TypeTableCapacity(usize),

    // Identifiers & literals
    Ident(String), IntLit(i64), FloatLit(f64), StrLit(String),

    // Punctuation & delimiters
    Colon, ColonAssign, Comma, Dot, Arrow,
    LBrace, RBrace, LParen, RParen, LBracket, RBracket,

    // Operators
    Assign, PlusAssign, MinusAssign,
    Plus, Minus, Star, Slash, Percent,
    EqEq, NotEq, Lt, LtEq, Gt, GtEq,

    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
    pub span: Span,
}

/// UTF-8 source lexer. `pos` is a byte offset; line/column are 1-based.
pub struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source, pos: 0, line: 1, col: 1 }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.advance();
                continue;
            }
            if ch == '#' {
                while let Some(c) = self.peek() {
                    if c == '\n' { break; }
                    self.advance();
                }
                continue;
            }

            let start = self.pos;
            let line = self.line;
            let col = self.col;
            let token = if ch == '"' {
                self.lex_string(start, line, col)?
            } else if ch.is_ascii_digit() {
                self.lex_number(start, line, col)?
            } else if ch.is_ascii_alphabetic() || ch == '_' {
                self.lex_identifier(start, line, col)?
            } else {
                self.lex_symbol(start, line, col)?
            };
            tokens.push(token);
        }
        tokens.push(Token {
            kind: TokenKind::Eof,
            line: self.line,
            col: self.col,
            span: Span { start: self.pos, end: self.pos },
        });
        Ok(tokens)
    }

    fn token(&self, kind: TokenKind, start: usize, line: usize, col: usize) -> Token {
        Token { kind, line, col, span: Span { start, end: self.pos } }
    }

    fn lex_string(&mut self, start: usize, line: usize, col: usize) -> Result<Token, String> {
        self.advance(); // opening quote
        let mut value = String::new();
        loop {
            match self.advance() {
                Some('"') => return Ok(self.token(TokenKind::StrLit(value), start, line, col)),
                Some('\\') => {
                    let escaped = self.advance().ok_or_else(|| format!("Unterminated escape at {}:{}", self.line, self.col))?;
                    match escaped {
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        '0' => value.push('\0'),
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        other => return Err(format!("Unknown escape sequence \\{} at {}:{}", other, self.line, self.col.saturating_sub(1))),
                    }
                }
                Some('\n') | None => return Err(format!("Unterminated string starting at {}:{}", line, col)),
                Some(ch) => value.push(ch),
            }
        }
    }

    fn lex_number(&mut self, start: usize, line: usize, col: usize) -> Result<Token, String> {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.advance(); }
        let mut is_float = false;
        if self.peek() == Some('.') && self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.advance();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) { self.advance(); }
        }
        let text = &self.source[start..self.pos];
        let kind = if is_float {
            TokenKind::FloatLit(text.parse::<f64>().map_err(|_| format!("Invalid float '{}' at {}:{}", text, line, col))?)
        } else {
            TokenKind::IntLit(text.parse::<i64>().map_err(|_| format!("Integer literal '{}' is out of range at {}:{}", text, line, col))?)
        };
        Ok(self.token(kind, start, line, col))
    }

    fn lex_identifier(&mut self, start: usize, line: usize, col: usize) -> Result<Token, String> {
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') { self.advance(); }
        let text = &self.source[start..self.pos];
        // T must be followed immediately by a positive decimal capacity: T32, T256, etc.
        let kind = if let Some(capacity_text) = text.strip_prefix('T').filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) {
            let capacity = capacity_text.parse::<usize>().map_err(|_| format!("Table capacity '{}' is out of range at {}:{}", capacity_text, line, col))?;
            if capacity == 0 { return Err(format!("Table capacity must be greater than zero at {}:{}", line, col)); }
            TokenKind::TypeTableCapacity(capacity)
        } else {
            match text {
                "const" => TokenKind::Const, "var" => TokenKind::Var,
                "fn" => TokenKind::Fn, "class" => TokenKind::Class,
                "comp" => TokenKind::Comp, "signal" => TokenKind::Signal,
                "from" => TokenKind::From, "use" => TokenKind::Use,
                "if" => TokenKind::If, "else" => TokenKind::Else,
                "for" => TokenKind::For, "in" => TokenKind::In,
                "while" => TokenKind::While, "return" => TokenKind::Return,
                "pass" => TokenKind::Pass, "true" => TokenKind::True,
                "break" => TokenKind::Break, "continue" => TokenKind::Continue,
                "false" => TokenKind::False, "and" => TokenKind::And,
                "or" => TokenKind::Or, "not" => TokenKind::Not,
                "lambda" => TokenKind::Lambda,
                "i" => TokenKind::TypeInt, "f" => TokenKind::TypeFloat,
                "s" => TokenKind::TypeStr, "bool" => TokenKind::TypeBool,
                "void" => TokenKind::TypeVoid,
                "T" => TokenKind::TypeTable,
                "vec2" => TokenKind::TypeVec2, "vec3" => TokenKind::TypeVec3,
                "vec4" => TokenKind::TypeVec4,
                _ => TokenKind::Ident(text.to_owned()),
            }
        };
        Ok(self.token(kind, start, line, col))
    }

    fn lex_symbol(&mut self, start: usize, line: usize, col: usize) -> Result<Token, String> {
        let ch = self.advance().expect("symbol was peeked");
        let kind = match ch {
            ':' => if self.consume_if('=') { TokenKind::ColonAssign } else { TokenKind::Colon }, ',' => TokenKind::Comma, '.' => TokenKind::Dot,
            '{' => TokenKind::LBrace, '}' => TokenKind::RBrace,
            '(' => TokenKind::LParen, ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket, ']' => TokenKind::RBracket,
            '*' => TokenKind::Star, '/' => TokenKind::Slash, '%' => TokenKind::Percent,
            '+' => if self.consume_if('=') { TokenKind::PlusAssign } else { TokenKind::Plus },
            '-' => if self.consume_if('>') { TokenKind::Arrow } else if self.consume_if('=') { TokenKind::MinusAssign } else { TokenKind::Minus },
            '=' => if self.consume_if('=') { TokenKind::EqEq } else { TokenKind::Assign },
            '!' => if self.consume_if('=') { TokenKind::NotEq } else { return Err(format!("Unexpected '!' at {}:{}; use 'not' or '!='", line, col)); },
            '<' => if self.consume_if('=') { TokenKind::LtEq } else { TokenKind::Lt },
            '>' => if self.consume_if('=') { TokenKind::GtEq } else { TokenKind::Gt },
            other => return Err(format!("Unexpected character '{}' at {}:{}", other, line, col)),
        };
        Ok(self.token(kind, start, line, col))
    }

    fn consume_if(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) { self.advance(); true } else { false }
    }
    fn peek(&self) -> Option<char> { self.source.get(self.pos..)?.chars().next() }
    fn peek_next(&self) -> Option<char> {
        let mut chars = self.source.get(self.pos..)?.chars();
        chars.next()?;
        chars.next()
    }
    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        if ch == '\n' { self.line += 1; self.col = 1; } else { self.col += 1; }
        Some(ch)
    }
}
