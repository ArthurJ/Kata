use crate::parser::ast::{Spanned, TypeRef, Pattern};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TExpr {
    Literal(TLiteral),
    Ident(String, TypeRef),
    Call(Box<Spanned<TExpr>>, Vec<Spanned<TExpr>>, TypeRef),
    Tuple(Vec<Spanned<TExpr>>, TypeRef),
    List(Vec<Spanned<TExpr>>, TypeRef),
    Lambda(Vec<Spanned<Pattern>>, Box<Spanned<TExpr>>, TypeRef),
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

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum TStmt {
    Let(Spanned<Pattern>, Spanned<TExpr>),
    Var(String, Spanned<TExpr>),
    Loop(Vec<Spanned<TStmt>>),
    For(String, Spanned<TExpr>, Vec<Spanned<TStmt>>),
    Match(Spanned<TExpr>, Vec<TMatchArm>),
    Expr(Spanned<TExpr>),
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TMatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Vec<Spanned<TStmt>>,
}
