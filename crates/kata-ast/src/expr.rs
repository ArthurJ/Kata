//! `Expr` — expressões da AST (saída do parser, entrada do typeck).
//!
//! A AST é plana e sem lógica — apenas dados. O typeck produz a TAST
//! (TypedExpr) a partir destes nós.

use crate::span::{Span, Spanned};

/// Uma expressão na AST.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // ── Literais ────────────────────────────────────────
    /// Literal inteiro. O texto bruto é preservado para BigInt/SMI no runtime.
    /// Suporta decimal, hex, oct, bin, separador `_`.
    IntLit { text: String },

    /// Literal float. Texto bruto preservado.
    FloatLit { text: String },

    /// Literal string. Conteúdo já unescaped.
    TextLit { text: String },

    /// `()` — unit literal.
    Unit,

    // ── Identificadores e aplicação ─────────────────────
    /// Identificador — referência a nome no escopo.
    /// Inclui operadores (`+`, `-`, `*`, `/`, `<`, `>`, `=`, `$`).
    Ident { name: String },

    /// Aplicação prefixa greedy: `f arg1 arg2 ...`.
    /// O parser coleta todos os argumentos que seguir o callee.
    Apply {
        callee: Box<Spanned<Expr>>,
        args: Vec<Spanned<Expr>>,
    },

    /// `expr::Type` — ascription de tipo.
    /// Em Fio 1: `3.14::Rational` (rebaixa FloatLit a Rational).
    /// Em Fio 6: `5::PositiveInt` (valida predicados, entrega refined).
    TypeAscription {
        expr: Box<Spanned<Expr>>,
        ty: Spanned<TypeExpr>,
    },

    /// `(expr)` — agrupamento (transparente ao typeck).
    /// Tem vírgula = Tuple; sem vírgula = Grouping.
    Grouping { inner: Box<Spanned<Expr>> },

    /// `(a, b, c)` — tupla heterogênea.
    /// `(42,)` é tupla de 1 elemento (vírgula obrigatória).
    /// `()` é Unit.
    Tuple { elements: Vec<Spanned<Expr>> },

    // ── Bindings ────────────────────────────────────────
    /// `let nome := expr` — binding imutável.
    Let {
        name: String,
        value: Box<Spanned<Expr>>,
    },

    // ── Qualificação de variante ────────────────────────
    /// `Enum::Variante` — qualificação de variante de enum.
    /// `Boolean::True`, `Result::Ok`, etc.
    /// O parser não sabe se `Boolean` é tipo ou módulo — produz
    /// `VariantQual` e o typeck resolve.
    VariantQual { enum_name: String, variant: String },
}

/// Item de top-level — declaração que aparece no nível de módulo.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    // ── Assinaturas de função ───────────────────────────
    /// `nome :: T1 T2 => TRet` — assinatura de função.
    /// Em Fio 1, usada para declarar operadores FFI no prelude:
    /// `+ :: Int Int => Int`
    /// Pode ter diretivas anexas (`@ffi`, `@associative`).
    Sig {
        name: String,
        params: Vec<Spanned<TypeExpr>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        body: Option<Spanned<Expr>>, // None para FFI (corpo suprido por @ffi)
    },

    // ── Declarações de tipo ─────────────────────────────
    /// `data Nome ()` — tipo opaco (sem campos).
    /// Em Fio 1: `data Int ()` com `@ffi("i64")`.
    /// Fio 5 trará campos: `data Pessoa (nome::Text idade::Int)`.
    DataDecl {
        name: String,
        fields: Vec<FieldDecl>, // vazio para tipos opacos de Fio 1
        directives: Vec<Directive>,
    },

    /// `enum Nome` com variantes indentadas.
    /// Em Fio 1: `enum Boolean { True, False }` — variantes unitárias.
    /// Fio 4 trará payload: `Ok(T)`, `Some(T)`.
    /// Fio 4 trará predicados: `Magreza(< _ 18.5)`.
    EnumDecl {
        name: String,
        variants: Vec<VariantDecl>,
        directives: Vec<Directive>,
    },

    // ── Expressão de entry point ────────────────────────
    /// Última expressão top-level — entry point implícito (I5).
    /// `+ 1 2` num arquivo é EntryExpr.
    EntryExpr(Spanned<Expr>),
}

/// Campo de struct: `nome::Tipo`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Spanned<TypeExpr>,
}

/// Variante de enum.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDecl {
    pub name: String,
    /// Payload da variante. None = unitária (`True`).
    /// Some(ty) = carrega tipo (`Ok(T)`).
    /// Predicados (Fio 4) não existem em Fio 1.
    pub payload: Option<Spanned<TypeExpr>>,
}

/// Diretiva `@nome`, `@nome("arg")`, `@nome{chave: valor}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<DirectiveArg>,
    /// Span do `@` para diagnósticos.
    pub span: Span,
}

/// Argumento de diretiva.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveArg {
    /// Argumento posicional: `@ffi("kata_rt_bi_add")`
    Str(String),
    /// Argumento posicional numérico: `@associative(0)`
    Int(i64),
    /// Argumento nomeado: `@cache_strategy{strategy: "LRU"}`
    Named { key: String, value: DirectiveValue },
}

/// Valor de argumento nomeado de diretiva.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveValue {
    Str(String),
    Int(i64),
}

/// Representação de tipo na AST (antes do typeck resolver para `Ty`).
/// O parser produz TypeExpr a partir de `::` em assinaturas e ascriptions.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// `Int`, `Float`, `Text`, `Rational`, `Boolean`, `Pessoa`, etc.
    /// Nome de tipo a ser resolvido no TypeEnv.
    Named(String),

    /// `()` — tipo Unit.
    Unit,

    /// `(T)` — tipo entre parênteses (agupamento de tipo).
    Grouping(Box<Spanned<TypeExpr>>),

    /// `Result::(T, E)` — tipo com parâmetros posicionais.
    /// O primeiro componente é o nome do tipo, os parênteses são os args.
    /// Em Fio 1 não usado (enums genéricos são Fio 4).
    ParamApp {
        name: String,
        params: Vec<Spanned<TypeExpr>>,
    },

    /// `(T1 -> T2)` — tipo de função como valor.
    /// Exige parênteses para desambiguar.
    Func {
        params: Vec<Spanned<TypeExpr>>,
        ret: Box<Spanned<TypeExpr>>,
    },
}

/// Um módulo completo — arquivo .kata.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub items: Vec<Spanned<Item>>,
}

impl Module {
    pub fn new(items: Vec<Spanned<Item>>) -> Self {
        Module { items }
    }
}
