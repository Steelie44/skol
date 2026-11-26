use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{Token, TokenKind};

/// Recursive-descent parser for Skol.
/// The parser owns its token stream and produces an AST without performing
/// type checking or other semantic validation (that lives in skol-sema).
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

/// How deeply expressions, blocks and types may nest. Keeps the parser, the
/// checker and the compiler (all recursive) well inside any thread's stack.
pub const MAX_NESTING: usize = 128;

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0, depth: 0 } }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(self.error(&format!("code is nested too deeply (more than {MAX_NESTING} levels)")));
        }
        Ok(())
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut items = Vec::new();
        while !self.is_at_end() {
            let before = self.pos;
            items.push(self.parse_top_item()?);
            if self.pos == before { return Err(self.error("Parser made no progress")); }
        }
        self.expect(TokenKind::Eof)?;
        Ok(Program { items })
    }

    fn parse_top_item(&mut self) -> Result<Item, String> {
        let start = self.mark();
        let kind = match self.peek_kind() {
            Some(TokenKind::From) => ItemKind::Import(self.parse_import()?),
            Some(TokenKind::Const) => ItemKind::Binding(self.parse_binding(false)?),
            Some(TokenKind::Var) => ItemKind::Binding(self.parse_binding(true)?),
            Some(TokenKind::Fn) => ItemKind::Fn(Rc::new(self.parse_fn()?)),
            Some(TokenKind::Class) => ItemKind::Class(Rc::new(self.parse_class()?)),
            Some(TokenKind::Comp) => ItemKind::Component(Rc::new(self.parse_component()?)),
            Some(TokenKind::Signal) => ItemKind::Signal(self.parse_signal()?),
            _ => ItemKind::Stmt(self.parse_stmt()?),
        };
        Ok(Item { kind, span: self.span_from(start) })
    }

    fn parse_import(&mut self) -> Result<ImportDecl, String> {
        self.expect(TokenKind::From)?;
        let module = self.expect_ident("Expected module name after 'from'")?;
        self.expect(TokenKind::Use)?;
        let mut names = vec![self.expect_ident("Expected imported name after 'use'")?];
        while self.match_kind(&TokenKind::Comma) {
            names.push(self.expect_ident("Expected imported name after ','")?);
        }
        Ok(ImportDecl { module, names })
    }

    fn parse_binding(&mut self, mutable: bool) -> Result<BindingDecl, String> {
        self.advance(); // const / var
        let name = self.expect_ident("Expected binding name")?;
        let (ty, init) = if self.match_kind(&TokenKind::Colon) {
            let ty = self.parse_type()?;
            self.expect(TokenKind::Assign)?;
            (ty, self.parse_expr()?)
        } else if self.match_kind(&TokenKind::ColonAssign) {
            let init = self.parse_lambda()?;
            let ty = match &init.kind {
                ExprKind::Lambda { params, ret_ty, .. } => TypeExpr::Function {
                    params: params.iter().map(|p| p.ty.clone()).collect(),
                    ret: Box::new(ret_ty.clone()),
                },
                _ => unreachable!("parse_lambda always returns Expr::Lambda"),
            };
            (ty, init)
        } else {
            return Err(self.error("Expected ':' with an explicit type or ':=' with a lambda"));
        };
        Ok(BindingDecl { mutable, name, ty, init })
    }

    fn parse_fn(&mut self) -> Result<FnDecl, String> {
        let start = self.mark();
        self.expect(TokenKind::Fn)?;
        let name = self.expect_ident("Expected function name")?;
        let params = self.parse_params()?;
        let ret_ty = if self.match_kind(&TokenKind::Arrow) { self.parse_type()? } else { TypeExpr::Void };
        let body = self.parse_block()?;
        Ok(FnDecl { name, params, ret_ty, body, span: self.span_from(start) })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, String> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let name = self.expect_ident("Expected parameter name")?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                params.push(Param { name, ty });
                if !self.match_kind(&TokenKind::Comma) { break; }
                if self.check(&TokenKind::RParen) { break; }
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(params)
    }

    fn parse_class(&mut self) -> Result<ClassDecl, String> {
        let start = self.mark();
        self.expect(TokenKind::Class)?;
        let name = self.expect_ident("Expected class name")?;
        // Composition syntax: class Enemy:(Health, Damage, Movement) { ... }
        let components = if self.match_kind(&TokenKind::Colon) { self.parse_component_names()? } else { Vec::new() };
        self.expect(TokenKind::LBrace)?;
        let (fields, methods) = self.parse_class_members()?;
        self.expect(TokenKind::RBrace)?;
        Ok(ClassDecl { name, components, fields, methods, span: self.span_from(start) })
    }

    fn parse_component_names(&mut self) -> Result<Vec<String>, String> {
        self.expect(TokenKind::LParen)?;
        let mut names = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                names.push(self.expect_ident("Expected component name")?);
                if !self.match_kind(&TokenKind::Comma) { break; }
                if self.check(&TokenKind::RParen) { break; }
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(names)
    }

    fn parse_component(&mut self) -> Result<ComponentDecl, String> {
        let start = self.mark();
        self.expect(TokenKind::Comp)?;
        let name = self.expect_ident("Expected component name")?;
        self.expect(TokenKind::LBrace)?;
        let (fields, methods) = self.parse_class_members()?;
        self.expect(TokenKind::RBrace)?;
        Ok(ComponentDecl { name, fields, methods, span: self.span_from(start) })
    }

    fn parse_class_members(&mut self) -> Result<(Vec<FieldDecl>, Vec<Rc<FnDecl>>), String> {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let before = self.pos;
            if self.check(&TokenKind::Fn) { methods.push(Rc::new(self.parse_fn()?)); }
            else {
                let start = self.mark();
                let name = self.expect_ident("Expected field name or method declaration")?;
                self.expect(TokenKind::Colon)?;
                let ty = self.parse_type()?;
                let default = if self.match_kind(&TokenKind::Assign) { Some(self.parse_expr()?) } else { None };
                fields.push(FieldDecl { name, ty, default, span: self.span_from(start) });
            }
            if self.pos == before { return Err(self.error("Parser made no progress in class body")); }
        }
        Ok((fields, methods))
    }

    fn parse_signal(&mut self) -> Result<SignalDecl, String> {
        self.expect(TokenKind::Signal)?;
        let name = self.expect_ident("Expected signal name")?;
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                params.push(self.expect_ident("Expected signal parameter name")?);
                if !self.match_kind(&TokenKind::Comma) { break; }
                if self.check(&TokenKind::RParen) { break; }
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(SignalDecl { name, params })
    }

    fn parse_type(&mut self) -> Result<TypeExpr, String> {
        self.enter()?;
        let r = self.parse_type_inner();
        self.depth -= 1;
        r
    }

    fn parse_type_inner(&mut self) -> Result<TypeExpr, String> {
        let token = self.peek_kind().cloned();
        match token {
            Some(TokenKind::TypeInt) => { self.advance(); Ok(TypeExpr::Int) }
            Some(TokenKind::TypeFloat) => { self.advance(); Ok(TypeExpr::Float) }
            Some(TokenKind::TypeStr) => { self.advance(); Ok(TypeExpr::Str) }
            Some(TokenKind::TypeBool) => { self.advance(); Ok(TypeExpr::Bool) }
            Some(TokenKind::TypeVoid) => { self.advance(); Ok(TypeExpr::Void) }
            Some(TokenKind::TypeVec2) => { self.advance(); Ok(TypeExpr::Vec2) }
            Some(TokenKind::TypeVec3) => { self.advance(); Ok(TypeExpr::Vec3) }
            Some(TokenKind::TypeVec4) => { self.advance(); Ok(TypeExpr::Vec4) }
            Some(TokenKind::TypeTableCapacity(capacity)) => {
                self.advance();
                self.expect(TokenKind::LBracket)?;
                let first = Box::new(self.parse_type()?);
                let ty = if self.match_kind(&TokenKind::Comma) {
                    let second = Box::new(self.parse_type()?);
                    TypeExpr::FixedTable2D { capacity, key: first, value: second }
                } else {
                    TypeExpr::FixedTable1D { capacity, element: first }
                };
                self.expect(TokenKind::RBracket)?;
                Ok(ty)
            }
            Some(TokenKind::TypeTable) => Err(self.error("Table types require a fixed capacity, for example T32[s]")),
            Some(TokenKind::Fn) => {
                // Function type: fn(i, s) -> bool   (no "-> T" means void)
                self.advance();
                self.expect(TokenKind::LParen)?;
                let mut params = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        params.push(self.parse_type()?);
                        if !self.match_kind(&TokenKind::Comma) { break; }
                        if self.check(&TokenKind::RParen) { break; }
                    }
                }
                self.expect(TokenKind::RParen)?;
                let ret = if self.match_kind(&TokenKind::Arrow) { self.parse_type()? } else { TypeExpr::Void };
                Ok(TypeExpr::Function { params, ret: Box::new(ret) })
            }
            Some(TokenKind::Ident(name)) => { self.advance(); Ok(TypeExpr::Named(name)) }
            other => Err(self.error(&format!("Expected type, found {:?}", other))),
        }
    }

    fn parse_block(&mut self) -> Result<Block, String> {
        self.enter()?;
        let r = self.parse_block_inner();
        self.depth -= 1;
        r
    }

    fn parse_block_inner(&mut self) -> Result<Block, String> {
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let before = self.pos;
            stmts.push(self.parse_stmt()?);
            if self.pos == before { return Err(self.error("Parser made no progress in block")); }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(Rc::new(stmts))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        let start = self.mark();
        let kind = match self.peek_kind() {
            Some(TokenKind::Const) => StmtKind::Binding(self.parse_binding(false)?),
            Some(TokenKind::Var) => StmtKind::Binding(self.parse_binding(true)?),
            Some(TokenKind::If) => self.parse_if()?,
            Some(TokenKind::For) => self.parse_for()?,
            Some(TokenKind::While) => self.parse_while()?,
            Some(TokenKind::Return) => {
                self.advance();
                if self.can_start_expr() { StmtKind::Return(Some(self.parse_expr()?)) }
                else { StmtKind::Return(None) }
            }
            Some(TokenKind::Pass) => { self.advance(); StmtKind::Pass }
            Some(TokenKind::Break) => { self.advance(); StmtKind::Break }
            Some(TokenKind::Continue) => { self.advance(); StmtKind::Continue }
            _ => {
                let target_or_expr = self.parse_expr()?;
                if let Some(op) = self.match_assign_op() {
                    let value = self.parse_expr()?;
                    StmtKind::Assign { target: target_or_expr, op, value }
                } else { StmtKind::Expr(target_or_expr) }
            }
        };
        Ok(Stmt { kind, span: self.span_from(start) })
    }

    fn parse_if(&mut self) -> Result<StmtKind, String> {
        self.expect(TokenKind::If)?;
        let cond = self.parse_expr()?;
        let then_body = self.parse_block()?;
        let else_body = if self.match_kind(&TokenKind::Else) {
            if self.check(&TokenKind::If) {
                let start = self.mark();
                self.enter()?;
                let kind = self.parse_if();
                self.depth -= 1;
                let kind = kind?;
                Some(Rc::new(vec![Stmt { kind, span: self.span_from(start) }]))
            } else { Some(self.parse_block()?) }
        } else { None };
        Ok(StmtKind::If { cond, then_body, else_body })
    }

    fn parse_for(&mut self) -> Result<StmtKind, String> {
        self.expect(TokenKind::For)?;
        let var = self.expect_ident("Expected loop variable name")?;
        self.expect(TokenKind::In)?;
        let iterable = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(StmtKind::For { var, iterable, body })
    }

    fn parse_while(&mut self) -> Result<StmtKind, String> {
        self.expect(TokenKind::While)?;
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(StmtKind::While { cond, body })
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let r = self.parse_or();
        self.depth -= 1;
        r
    }
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_and()?;
        let mut levels = 0;
        while self.match_kind(&TokenKind::Or) {
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_and() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, BinaryOp::Or, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_equality()?;
        let mut levels = 0;
        while self.match_kind(&TokenKind::And) {
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_equality() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, BinaryOp::And, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_comparison()?;
        let mut levels = 0;
        loop {
            let op = match self.peek_kind() { Some(TokenKind::EqEq) => BinaryOp::Eq, Some(TokenKind::NotEq) => BinaryOp::NotEq, _ => break };
            self.advance();
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_comparison() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, op, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_term()?;
        let mut levels = 0;
        loop {
            let op = match self.peek_kind() { Some(TokenKind::Lt) => BinaryOp::Lt, Some(TokenKind::LtEq) => BinaryOp::LtEq, Some(TokenKind::Gt) => BinaryOp::Gt, Some(TokenKind::GtEq) => BinaryOp::GtEq, _ => break };
            self.advance();
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_term() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, op, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_factor()?;
        let mut levels = 0;
        loop {
            let op = match self.peek_kind() { Some(TokenKind::Plus) => BinaryOp::Add, Some(TokenKind::Minus) => BinaryOp::Sub, _ => break };
            self.advance();
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_factor() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, op, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_factor(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        let mut levels = 0;
        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::Star) => BinaryOp::Mul,
                Some(TokenKind::Slash) => BinaryOp::Div,
                Some(TokenKind::Percent) => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            levels += 1;
            if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            let rhs = match self.parse_unary() { Ok(r) => r, Err(e) => { self.depth -= levels; return Err(e); } };
            expr = binary(expr, op, rhs);
        }
        self.depth -= levels;
        Ok(expr)
    }
    fn parse_unary(&mut self) -> Result<Expr, String> {
        let start = self.mark();
        if self.match_kind(&TokenKind::Not) {
            self.enter()?;
            let expr = self.parse_unary();
            self.depth -= 1;
            let expr = expr?;
            return Ok(self.node(ExprKind::Unary { op: UnaryOp::Not, expr: Box::new(expr) }, start));
        }
        if self.match_kind(&TokenKind::Minus) {
            self.enter()?;
            let expr = self.parse_unary();
            self.depth -= 1;
            let expr = expr?;
            return Ok(self.node(ExprKind::Unary { op: UnaryOp::Neg, expr: Box::new(expr) }, start));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let start = self.mark();
        let mut expr = self.parse_primary()?;
        let mut levels = 0;
        loop {
            if matches!(self.peek_kind(), Some(TokenKind::Dot | TokenKind::LParen | TokenKind::LBracket)) {
                levels += 1;
                if let Err(e) = self.enter() { self.depth -= levels; return Err(e); }
            }
            if self.match_kind(&TokenKind::Dot) {
                let member = self.expect_ident("Expected member or method name after '.'")?;
                if self.match_kind(&TokenKind::LParen) {
                    let args = self.parse_arg_list()?;
                    expr = self.node(ExprKind::MethodCall { object: Box::new(expr), method: member, args }, start);
                } else {
                    expr = self.node(ExprKind::Field { object: Box::new(expr), field: member }, start);
                }
            } else if self.match_kind(&TokenKind::LParen) {
                let args = self.parse_arg_list()?;
                expr = self.node(ExprKind::Call { callee: Box::new(expr), args }, start);
            } else if self.match_kind(&TokenKind::LBracket) {
                let index = self.parse_expr()?;
                self.expect(TokenKind::RBracket)?;
                expr = self.node(ExprKind::Index { object: Box::new(expr), index: Box::new(index) }, start);
            } else { break; }
        }
        self.depth -= levels;
        Ok(expr)
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop { args.push(self.parse_expr()?); if !self.match_kind(&TokenKind::Comma) { break; } if self.check(&TokenKind::RParen) { break; } }
        }
        self.expect(TokenKind::RParen)?;
        Ok(args)
    }

    fn parse_lambda(&mut self) -> Result<Expr, String> {
        let start = self.mark();
        self.expect(TokenKind::Lambda)?;
        let params = self.parse_params()?;
        self.expect(TokenKind::Arrow)?;
        let ret_ty = self.parse_type()?;
        let body = self.parse_block()?;
        Ok(self.node(ExprKind::Lambda { params, ret_ty, body }, start))
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let start = self.mark();
        let kind = match self.peek_kind().cloned() {
            Some(TokenKind::IntLit(n)) => { self.advance(); ExprKind::IntLit(n) }
            Some(TokenKind::FloatLit(n)) => { self.advance(); ExprKind::FloatLit(n) }
            Some(TokenKind::StrLit(s)) => { self.advance(); ExprKind::StrLit(s) }
            Some(TokenKind::True) => { self.advance(); ExprKind::BoolLit(true) }
            Some(TokenKind::False) => { self.advance(); ExprKind::BoolLit(false) }
            Some(TokenKind::Ident(name)) => { self.advance(); ExprKind::Ident(name) }
            Some(TokenKind::LParen) => {
                self.advance();
                let mut expr = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                expr.span = self.span_from(start);
                return Ok(expr);
            }
            Some(TokenKind::LBrace) => return self.parse_table_lit(),
            Some(TokenKind::Lambda) => return self.parse_lambda(),
            other => return Err(self.error(&format!("Unexpected token in expression: {:?}", other))),
        };
        Ok(self.node(kind, start))
    }

    fn parse_table_lit(&mut self) -> Result<Expr, String> {
        let start = self.mark();
        self.expect(TokenKind::LBrace)?;
        let mut entries = Vec::new();
        if !self.check(&TokenKind::RBrace) {
            loop {
                let first = self.parse_expr()?;
                if self.match_kind(&TokenKind::Colon) { let second = self.parse_expr()?; entries.push(TableEntry::Pair(first, second)); }
                else { entries.push(TableEntry::Item(first)); }
                if !self.match_kind(&TokenKind::Comma) { break; }
                if self.check(&TokenKind::RBrace) { break; }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(self.node(ExprKind::TableLit(entries), start))
    }

    fn node(&self, kind: ExprKind, start: usize) -> Expr { Expr { kind, span: self.span_from(start) } }

    fn can_start_expr(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::IntLit(_) | TokenKind::FloatLit(_) | TokenKind::StrLit(_) | TokenKind::True | TokenKind::False | TokenKind::Ident(_) | TokenKind::LParen | TokenKind::LBrace | TokenKind::Minus | TokenKind::Not | TokenKind::Lambda))
    }
    fn match_assign_op(&mut self) -> Option<AssignOp> {
        let op = match self.peek_kind() { Some(TokenKind::Assign) => AssignOp::Assign, Some(TokenKind::PlusAssign) => AssignOp::PlusAssign, Some(TokenKind::MinusAssign) => AssignOp::MinusAssign, _ => return None };
        self.advance(); Some(op)
    }
    fn expect_ident(&mut self, msg: &str) -> Result<String, String> {
        match self.peek_kind().cloned() { Some(TokenKind::Ident(s)) => { self.advance(); Ok(s) }, _ => Err(self.error(msg)) }
    }
    fn expect(&mut self, expected: TokenKind) -> Result<(), String> {
        if self.check(&expected) { self.advance(); Ok(()) }
        else { Err(self.error(&format!("Expected {:?}, found {:?}", expected, self.peek_kind()))) }
    }
    fn check(&self, kind: &TokenKind) -> bool { self.peek_kind() == Some(kind) }
    fn match_kind(&mut self, kind: &TokenKind) -> bool { if self.check(kind) { self.advance(); true } else { false } }
    fn peek(&self) -> Option<&Token> { self.tokens.get(self.pos) }
    fn peek_kind(&self) -> Option<&TokenKind> { self.peek().map(|t| &t.kind) }
    fn advance(&mut self) -> Option<&Token> { if self.pos < self.tokens.len() { let i = self.pos; self.pos += 1; self.tokens.get(i) } else { None } }
    fn is_at_end(&self) -> bool { matches!(self.peek_kind(), None | Some(TokenKind::Eof)) }
    /// Byte offset where the next construct starts.
    fn mark(&self) -> usize { self.peek().map(|t| t.span.start).unwrap_or(0) }
    /// Span from `start` to the end of the last consumed token.
    fn span_from(&self, start: usize) -> Span {
        let end = if self.pos > 0 { self.tokens[self.pos - 1].span.end } else { start };
        Span::new(start, end.max(start))
    }
    fn error(&self, message: &str) -> String {
        match self.peek() { Some(t) => format!("Parse error at {}:{} (bytes {}..{}): {}", t.line, t.col, t.span.start, t.span.end, message), None => format!("Parse error at EOF: {}", message) }
    }
}

fn binary(left: Expr, op: BinaryOp, right: Expr) -> Expr {
    let span = Span::new(left.span.start, right.span.end);
    Expr { kind: ExprKind::Binary { left: Box::new(left), op, right: Box::new(right) }, span }
}
