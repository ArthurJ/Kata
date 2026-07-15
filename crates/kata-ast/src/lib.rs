//! AST de dados puros — sem lógica, sem dependência do compilador.
//!
//! Define [`Span`], [`Spanned`], [`Token`], [`Expr`], [`Item`], [`TypeExpr`],
//! [`Directive`] e [`Module`] — estruturas de dados que o lexer/parser
//! produzem e o typeck consome.
//!
//! Esta crate é uma leaf crate: não depende de nenhum outro crate do projeto.
//! `kata-core` depende de `kata-ast` para ter acesso a `Span` (orphan rule, I7).

pub(crate) mod expr;
pub(crate) mod span;
pub(crate) mod token;

pub use expr::{
    ActionStmt, Directive, DirectiveArg, DirectiveValue, DotIndex, ExportItem, Expr, FieldDecl,
    GuardClause, ImplMethod, InterfaceSig, Item, LambdaClause, MatchArm, Module, Pattern,
    RefinedDecl, TypeExpr, VariantDecl, WithBinding,
};
pub use span::{Span, Spanned};
pub use token::{Token, TokenWithSpan};
