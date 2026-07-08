//! `TypedExpr` — TAST enriquecida (saída do typeck, entrada do codegen).
//!
//! Cada nó carrega `ty: Ty`, `tail_pos: bool`, `effect: Effect` — os três
//! populados desde Fio 1 para evitar retrofit (PRD-fio1 §17, Risco 5).
//!
//! `TypedExprKind` espelha `Expr` mas com `Spanned<TypedExpr>` em vez de
//! `Spanned<Expr>` — a recursão é sobre a TAST, não sobre a AST.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::ty::{Ty, TypeEnv};

/// Efeito de uma expressão. Fio 1 só produz `Puro`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Expressão pura — sem efeitos colaterais (Fio 1).
    Puro,
    /// Efeito de I/O — `echo`, leitura/escrita (Fio 3).
    IO,
    /// Spawn de fiber — `spawn` (Fio 11).
    Spawn,
    /// Operação de canal — `send`/`recv` (Fio 11).
    ChannelOp,
}

/// Nó da TAST — expressão com tipo, posição de cauda e efeito anotados.
#[derive(Debug, Clone)]
pub struct TypedExpr {
    /// Span herdado do nó da AST correspondente.
    pub span: Span,
    /// Tipo canônico inferido pelo typeck.
    pub ty: Ty,
    /// `true` se a expressão está em posição de cauda (última expr de um
    /// bloco/entry). Marcado em toda expr — o codegen usa para otimizar
    /// tail calls (Fio 9+) e o TRMA pass (fios posteriores).
    pub tail_pos: bool,
    /// Efeito da expressão. `Puro` em Fio 1.
    pub effect: Effect,
    /// Variante da TAST — espelha `Expr` com nós filhos já tipados.
    pub kind: TypedExprKind,
}

/// Variante da TAST. Espelha `kata_ast::Expr` mas recursão é sobre
/// `Spanned<TypedExpr>`.
#[derive(Debug, Clone)]
pub enum TypedExprKind {
    /// Literal inteiro. Texto bruto preservado para BigInt/SMI no runtime.
    IntLit { text: String },
    /// Literal float. Texto bruto preservado.
    FloatLit { text: String },
    /// Literal string. Conteúdo já unescaped.
    TextLit { text: String },
    /// `()` — unit literal.
    Unit,
    /// Identificador — referência a nome no escopo. O tipo vem do TypeEnv.
    Ident { name: String },
    /// Aplicação prefixa greedy. O `dispatch` já resolveu a sobrecarga —
    /// `ffi_symbol` e `ret` estão disponíveis para o codegen.
    Apply {
        callee: Box<Spanned<TypedExpr>>,
        args: Vec<Spanned<TypedExpr>>,
        /// Símbolo FFI resolvido pelo DispatchTable (ex: `kata_rt_bi_add`).
        /// `None` para funções Kata puras (corpo no próprio módulo).
        ffi_symbol: Option<String>,
    },
    /// `expr::Type` — ascription de tipo. O typeck validou compatibilidade
    /// ou rebaixou (FloatLit → Rational).
    TypeAscription {
        expr: Box<Spanned<TypedExpr>>,
        /// Tipo alvo da ascription (já resolvido de TypeExpr → Ty).
        target_ty: Ty,
    },
    /// `(expr)` — agrupamento (transparente ao codegen).
    Grouping { inner: Box<Spanned<TypedExpr>> },
    /// Tupla heterogênea. **Não suportada em Fio 1** — o typeck rejeita.
    /// `Ty::Tuple` não existe ainda (Fio 5). Presente na TAST apenas para
    /// que o typeck produza um erro limpo em vez de panic no match.
    Tuple { elements: Vec<Spanned<TypedExpr>> },
    /// `let nome := expr` — binding imutável. O typeck definiu `nome` no
    /// TypeEnv com `ty = expr.ty`. `tail_pos = false` sempre.
    Let {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },
    /// `Enum::Variante` — qualificação de variante de enum.
    /// `Boolean::True` → `Ty::Sum("Boolean")`.
    VariantQual { enum_name: String, variant: String },
}

/// Módulo tipado — artefato final do Pass 2.
///
/// Contém a TAST do entry point e o DispatchTable populado com todas as
/// assinaturas (prelude + módulo do usuário). O codegen consome isto.
#[derive(Debug, Clone)]
pub struct TypedModule {
    /// Entry point tipado — última expressão top-level do módulo.
    pub entry: Spanned<TypedExpr>,
    /// DispatchTable populado com prelude + assinaturas do módulo.
    pub dispatch_table: DispatchTable,
    /// Snapshot do TypeEnv ao final do typeck (para inspeção/debug).
    pub type_env: TypeEnv,
}
