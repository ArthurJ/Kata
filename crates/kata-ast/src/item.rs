//! `Item` — declarações de nível superior (saída do parser, entrada do typeck).
//!
//! Contém o enum `Item` (assinaturas de função, declarações de tipo, actions,
//! interfaces, implements, import/export, entry expr) e todos os structs
//! auxiliares que essas declarações referenciam.

use crate::expr::{Expr, LambdaClause, TypeExpr};
use crate::span::{Span, Spanned};

/// Item de top-level — declaração que aparece no nível de módulo.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    // ── Assinaturas de função ───────────────────────────
    /// `nome :: T1 T2 => TRet` — assinatura de função.
    /// Usada para declarar operadores FFI no prelude:
    /// `+ :: Int Int => Int`
    /// Pode ter diretivas anexas (`@ffi`, `@associative`).
    Sig {
        name: String,
        params: Vec<Spanned<TypeExpr>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        // Sempre None (FFI — corpo suprido por @ffi).
        // Some(clauses) = função pura com corpo Kata.
        body: Option<Vec<Spanned<LambdaClause>>>,
    },

    // ── Declarações de tipo ─────────────────────────────
    /// `data Nome ()` — tipo opaco (sem campos).
    /// `data Int ()` com `@ffi("i64")`.
    /// Trará campos: `data Pessoa (nome::Text idade::Int)`.
    /// Trará refined: `data (Int, > _ 0) as PositiveInt`.
    DataDecl {
        name: String,
        fields: Vec<FieldDecl>, // vazio para tipos opacos
        directives: Vec<Directive>,
        /// Refined declaration. None = struct normal.
        /// Some(RefinedDecl) = tipo refinado com predicados.
        refined: Option<RefinedDecl>,
    },

    /// `enum Nome` com variantes indentadas.
    /// `enum Boolean { True, False }` — variantes unitárias.
    /// Trará payload: `Ok(T)`, `Some(T)`.
    /// Trará predicados: `Magreza(< _ 18.5)`.
    EnumDecl {
        name: String,
        variants: Vec<VariantDecl>,
        directives: Vec<Directive>,
    },

    // ── Aliases ──────────────────────────────────
    /// `alias Target as NewName` — cria um newtype (tipo nominal distinto
    /// com o mesmo layout do target). O construtor sintetizado é identity:
    /// `NewName :: Target => NewName`.
    AliasDecl { target: String, new_name: String },

    // ── Actions ─────────────────────────────────
    /// `action nome` com body indentado.
    /// Actions são o domínio impuro. Declaração com `action nome` (sem `!`).
    /// O body é uma sequência de statements (`ActionStmt`).
    ActionDecl {
        name: String,
        /// Parâmetros da Action (uma tupla tipada, ou vazia).
        params: Vec<Spanned<TypeExpr>>,
        /// Nomes dos params. `Some(nome)` se o param é nomeado (`x::Tipo`),
        /// `None` se posicional (legado — não usado após migração total).
        param_names: Vec<Option<String>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        /// Body da Action (statements sequenciais).
        body: Vec<crate::expr::ActionStmt>,
    },

    // ── Interfaces, Implements, Import, Export ──────────
    /// `interface NOME implements SUPER1 SUPER2 ...` + bloco indentado
    /// de assinaturas obrigatórias.
    InterfaceDecl {
        name: String,
        supertraits: Vec<String>,
        /// Type params da interface (ex: `A` em `ITERABLE(A)`).
        type_params: Vec<String>,
        signatures: Vec<InterfaceSig>,
    },

    /// `Tipo implements Interface` + bloco indentado com métodos.
    ImplementsDecl {
        type_name: String,
        /// Type params do tipo (ex: `A` em `List::(A) implements ITERABLE::(A)`).
        type_params: Vec<String>,
        interface_name: String,
        /// Params da interface vinculados (ex: `A` em `ITERABLE(A)`).
        iface_params: Vec<String>,
        /// Métodos: assinaturas concretas + corpo (lambda ou @ffi).
        methods: Vec<ImplMethod>,
    },

    /// `import modulo.submodulo` / `import ... as alias` /
    /// `import MOD.(items)`.
    ImportDecl {
        path: Vec<String>,
        alias: Option<String>,
        items: Option<Vec<String>>, // None = tudo, Some = seletivo
    },

    /// `export item1 item2 ...` / `export MOD.(itens)`.
    ExportDecl { items: Vec<ExportItem> },

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

/// Declaração de tipo refinado: `data (Int, > _ 0) as PositiveInt`.
/// O conteúdo de `()` é um TypeExpr (base) seguido de predicados (Expr).
/// Cada predicado usa `Hole` (`_`) como placeholder para o valor a ser validado.
#[derive(Debug, Clone, PartialEq)]
pub struct RefinedDecl {
    /// Tipo base: `Int`, `Float`, etc.
    pub base_ty: Spanned<TypeExpr>,
    /// Predicados: `> _ 0`, `<= _ 100`, etc. AND lógico — todos devem passar.
    pub predicates: Vec<Spanned<Expr>>,
}

/// Variante de enum.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDecl {
    pub name: String,
    /// Payload da variante. None = unitária (`True`).
    /// Some(ty) = carrega tipo (`Ok(T)`).
    pub payload: Option<Spanned<TypeExpr>>,
    /// Predicado da variante. None = sem predicado.
    /// `Magreza(< _ 18.5)` → predicate = Some(Apply { <, [Hole, 18.5] }).
    /// `Obesidade` → predicate = None (default/fallback).
    pub predicate: Option<Spanned<Expr>>,
    /// Valor fixo constante. None = não é constante.
    /// `OK(200)` → fixed_value = Some(IntLit(200)).
    /// Variante constante: `OK` sem args constrói com este valor.
    /// Passar args a uma variante constante é erro de tipo.
    pub fixed_value: Option<Spanned<Expr>>,
}

/// Diretiva `@nome`, `@nome("arg")`, `@nome{chave: valor}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<DirectiveArg>,
    /// Span do `@` para diagnósticos.
    pub span: Span,
}

/// Argumento de diretiva — posicional ou nomeado. Ambos carregam `Expr`
/// (não um enum restrito), para que valores de diretiva usem a mesma sintaxe
/// que o resto da linguagem: tupla, variant, apply posicional de construtor,
/// etc. Cada consumer (`@ffi`, `@test`) valida o tipo de `Expr` que espera.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveArg {
    /// Argumento posicional: `@ffi("kata_rt_bi_add")`, `@associative(0)`.
    /// O `Expr` é tipicamente `TextLit` ou `IntLit`, mas o parser não impõe —
    /// o consumer valida.
    Expr(Box<Spanned<crate::expr::Expr>>),
    /// Argumento nomeado: `@test{desc: "...", args: (1, 2)}`.
    Named {
        key: String,
        value: Box<Spanned<crate::expr::Expr>>,
    },
}

/// Assinatura dentro de interface — sem corpo, sem diretivas.
/// `+ :: NUM NUM => NUM`
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceSig {
    pub name: String,
    pub params: Vec<Spanned<TypeExpr>>,
    pub ret: Spanned<TypeExpr>,
}

/// Método dentro de implements — assinatura + corpo.
/// `+ :: Complex Complex => Complex` + lambda ou @ffi.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplMethod {
    pub name: String,
    pub params: Vec<Spanned<TypeExpr>>,
    pub ret: Spanned<TypeExpr>,
    pub directives: Vec<Directive>,
    /// None = FFI (precisa @ffi); Some = corpo Kata (cláusulas lambda).
    pub body: Option<Vec<Spanned<LambdaClause>>>,
}

/// Item de export: nome simples ou reexportação de submódulo.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportItem {
    /// Nome do item exportado (ex: `+`, `TipoX`, `NUM`).
    pub name: String,
    /// Reexportação: `MOD.(itens)` — None = export direto,
    /// Some = reexportar itens de outro módulo.
    pub reexport_from: Option<String>,
    /// Itens reexportados (quando reexport_from é Some).
    pub reexport_items: Option<Vec<String>>,
}
