//! Patterns e cláusulas tipadas — extrato de `typed.rs`.
//!
//! Tipos que descrevem padrões (`TypedPattern`) e cláusulas de match/lambda
//! (`TypedMatchArm`, `TypedGuardClause`, `TypedWithBinding`,
//! `TypedLambdaClause`). Cohesão: todos descrevem a estrutura de matching
//! após o typeck resolver tipos de bindings.

use kata_ast::Spanned;
use kata_core::ty::Ty;

use crate::typed::TypedExpr;

/// Pattern tipado — pattern da AST com tipo resolvido para bindings.
///
/// O typeck resolve `Pattern::Ident("True")` para `TypedPattern::Variant`
/// se `True` é variante do enum do scrutinee. Para `Ident("x")` que não é
/// variante, o typeck mantém `Ident` e liga `x` ao tipo do scrutinee.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TypedPattern {
    /// `x` — liga o valor ao nome. `ty` é o tipo do valor ligado.
    Ident { name: String, ty: Ty },
    /// `_` — wildcard, aceita qualquer valor sem ligar nome.
    Wildcard,
    /// `42`, `"texto"`, `3.14` — literal exato. O TypedExpr já é tipado.
    Literal { value: Spanned<TypedExpr> },
    /// `Boolean::True`, `Result::Ok` — variante de enum.
    /// Resolvido pelo typeck a partir de `Pattern::Ident("True")` ou
    /// `Pattern::Variant { enum_name, variant }`.
    /// `sub_patterns` é None para variantes unitárias (`True`, `False`).
    /// Some(vec) para variantes com payload (`Ok(v)`, `Some(x)`).
    /// `tag` é o índice da variante no enum (para codegen de match).
    Variant {
        enum_name: String,
        variant: String,
        /// Sub-patterns do payload. None = unitária.
        /// Some(vec) = variante com payload (1 sub-pattern por enquanto).
        sub_patterns: Option<Vec<Spanned<TypedPattern>>>,
        /// Índice da variante no enum (tag do Sum no codegen).
        tag: usize,
    },
    /// `(a, b, c)` — tupla. Cada sub-pattern é tipado recursivamente.
    Tuple {
        elements: Vec<Spanned<TypedPattern>>,
    },
    /// `[h : t]` — cons (stub em Fio 2 — List é Fio 8).
    Cons {
        head: Box<Spanned<TypedPattern>>,
        tail: Box<Spanned<TypedPattern>>,
    },
}

/// Braço de match tipado.
#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    /// Pattern resolvido. None = `otherwise` (fallback).
    pub pattern: Option<Spanned<TypedPattern>>,
    /// Guard opcional após pattern.
    pub guard: Option<Spanned<TypedExpr>>,
    /// Corpo do braço.
    pub body: Spanned<TypedExpr>,
}

/// Guard tipado: `condição: corpo` ou `otherwise: corpo`.
#[derive(Debug, Clone)]
pub struct TypedGuardClause {
    /// None = `otherwise` (sempre passa).
    pub condition: Option<Spanned<TypedExpr>>,
    pub body: Spanned<TypedExpr>,
}

/// Binding de `with` tipado: `nome := expr`.
#[derive(Debug, Clone)]
pub struct TypedWithBinding {
    pub name: String,
    pub value: Spanned<TypedExpr>,
}

/// Cláusula lambda tipada — padrões + corpo, com guards e with bindings.
#[derive(Debug, Clone)]
pub struct TypedLambdaClause {
    /// Padrões já tipados (com tipo de cada binding).
    pub patterns: Vec<Spanned<TypedPattern>>,
    /// Corpo da cláusula (quando não há guards).
    pub body: Spanned<TypedExpr>,
    /// Guards opcionais. Se não-vazio, o corpo é decidido pelos guards.
    pub guards: Vec<TypedGuardClause>,
    /// `with` bindings (açúcar → `let` chain, já resolvidos).
    pub with_bindings: Vec<TypedWithBinding>,
}
