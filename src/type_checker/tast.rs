use crate::parser::ast::{Spanned, TypeRef, Pattern};

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum AllocMode {
    Local,
    Shared,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TExpr {
    Literal(TLiteral),
    Ident(String, TypeRef),
    Call(Box<Spanned<TExpr>>, Vec<Spanned<TExpr>>, TypeRef),
    Tuple(Vec<Spanned<TExpr>>, TypeRef, AllocMode),
    List(Vec<Spanned<TExpr>>, TypeRef, AllocMode),
    Array(Vec<Vec<Spanned<TExpr>>>, TypeRef, AllocMode),
    Lambda(Vec<Spanned<Pattern>>, Box<Spanned<TExpr>>, TypeRef, AllocMode),
    Sequence(Vec<Spanned<TExpr>>, TypeRef),
    Guard(Vec<(Spanned<TExpr>, Spanned<TExpr>)>, Box<Spanned<TExpr>>, TypeRef),
    Try(Box<Spanned<TExpr>>, TypeRef),
    ChannelSend(Box<Spanned<TExpr>>, Box<Spanned<TExpr>>, TypeRef),
    ChannelRecv(Box<Spanned<TExpr>>, TypeRef),
    ChannelRecvNonBlock(Box<Spanned<TExpr>>, TypeRef),
    Hole,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TSelectArm {
    pub operation: Spanned<TExpr>,
    pub binding: Option<Spanned<Pattern>>,
    pub body: Vec<Spanned<TStmt>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TStmt {
    Let(Spanned<Pattern>, Spanned<TExpr>),
    Var(String, Spanned<TExpr>),
    Loop(Vec<Spanned<TStmt>>),
    For(String, Spanned<TExpr>, Vec<Spanned<TStmt>>),
    Match(Spanned<TExpr>, Vec<TMatchArm>),
    Select(Vec<TSelectArm>, Option<(Spanned<TExpr>, Vec<Spanned<TStmt>>)>),
    ActionAssign(Spanned<TExpr>, Spanned<TExpr>),
    Expr(Spanned<TExpr>),
    Break,
    Continue,
    DropShared(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TMatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Vec<Spanned<TStmt>>,
}
