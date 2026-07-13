//! `TypedExpr` — TAST enriquecida (saída do typeck, entrada do codegen).
//!
//! Cada nó carrega `ty: Ty`, `tail_pos: bool`, `effect: Effect` — os três
//! populados desde Fio 1 para evitar retrofit (PRD-fio1 §17, Risco 5).
//!
//! `TypedExprKind` espelha `Expr` mas com `Spanned<TypedExpr>` em vez de
//! `Spanned<Expr>` — a recursão é sobre a TAST, não sobre a AST.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::escape::EscapeTarget;
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
    /// Destino de escape para seleção de arena (Pré-11).
    /// Generaliza `tail_pos` para memória: `Local` = fiber_arena,
    /// `Caller` = caller_arena, `Ancestor(n)` = arena do LCA.
    /// Coexiste com `tail_pos` (que governa TCO, não memória).
    pub escape: EscapeTarget,
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
    /// Chamada de função na TAST. Renomeado de `Apply` em Fio 2 (PRD-fio2).
    ///
    /// As variáveis capturadas pelo callee são lidas de `Lambda.captures`
    /// (single source of truth). O call site aloca um CaptureBox via
    /// `kata_rt_alloc_arc` e passa `box_ptr` como primeiro arg.
    Closure {
        callee: Box<Spanned<TypedExpr>>,
        args: Vec<Spanned<TypedExpr>>,
        /// Símbolo FFI resolvido pelo DispatchTable (ex: `kata_rt_bi_add`).
        /// `None` para funções Kata puras (corpo no próprio módulo) ou
        /// para `call_indirect` (callee é variável com `Ty::Function`).
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
    /// Tupla heterogênea. Em Fio 2, aceita para patterns (Ty::Tuple antecipado).
    Tuple { elements: Vec<Spanned<TypedExpr>> },
    /// `let nome := expr` — binding imutável. O typeck definiu `nome` no
    /// TypeEnv com `ty = expr.ty`. `tail_pos = false` sempre.
    Let {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },
    /// `Enum::Variante` — qualificação de variante de enum unitária.
    /// `Boolean::True` → `Ty::Sum("Boolean")`.
    /// Só usado para variantes sem payload.
    /// `tag` é o índice da variante no enum (para codegen de Sum não-Boolean).
    VariantQual {
        enum_name: String,
        variant: String,
        tag: usize,
    },
    /// `Enum::Variante payload` — construção de variante com payload.
    /// `Result::Ok 42` → `Ty::Sum("Result")` com payload = 42.
    /// Fase 5 — Sum com payload.
    /// `tag` é o índice da variante no enum (para codegen).
    VariantConstruct {
        enum_name: String,
        variant: String,
        payload: Box<Spanned<TypedExpr>>,
        tag: usize,
    },

    // ── Fio 2: Lambda, Match ──────────────────────────────────────
    /// Lambda — função pura com corpo Kata.
    /// Pode ser anônimo (em posição de expressão) ou nomeado (cláusulas de Sig).
    Lambda {
        /// Nome da função no JITModule (para call direto).
        /// None para lambda anônimo ainda não compilado como função separada.
        func_name: Option<String>,
        /// Tipos dos parâmetros (da assinatura ou inferidos dos padrões).
        param_types: Vec<Ty>,
        /// Tipo de retorno.
        ret_ty: Ty,
        /// Cláusulas (padrões + corpo). 1 cláusula = lambda anônimo.
        /// Múltiplas = função nomeada.
        clauses: Vec<TypedLambdaClause>,
        /// Variáveis capturadas do escopo externo (Fase 12).
        /// Populado por collect_captures. O codegen aloca um CaptureBox
        /// (via `kata_rt_alloc_arc`) e passa `box_ptr` como primeiro arg
        /// da função JIT. Sempre Heap — sem escape analysis.
        captures: Vec<CaptureInfo>,
    },

    /// Match — pattern matching com verificação de exaustividade.
    Match {
        scrutinee: Box<Spanned<TypedExpr>>,
        arms: Vec<TypedMatchArm>,
    },

    // ── Fio 3: Actions, var ──────────────────────────────────────
    /// Chamada de Action (`nome!(args)`).
    /// O codegen emite call para a função Cranelift da Action, passando
    /// caller_arena handle como primeiro parâmetro implícito.
    ActionCall {
        callee: String,
        args: Box<Spanned<TypedExpr>>,
        /// caller_arena handle para a Action chamada alocar retornos.
        /// É o local_arena do caller (que se torna caller_arena do callee).
        caller_arena: i64,
        /// Símbolo FFI se a Action é builtin (ex: "kata_rt_print" para echo).
        /// None para Actions definidas pelo usuário (despacha via kata_refs).
        ffi_symbol: Option<String>,
    },

    /// `var nome := expr` — binding mutável.
    /// Semântica igual a `Let` na TAST (o codegen trata a mutabilidade).
    Var {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },

    /// `nome := expr` — reatribuição a variável `var` (exclusivo de Actions).
    /// O typeck verificou que `name` foi declarado como mutável e que o tipo
    /// do valor é compatível. O codegen faz `def_var` com o novo valor.
    Reassign {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },

    /// `return expr` — early return de uma Action.
    /// O typeck verificou que o tipo de `expr` bate com `ret_ty` da Action.
    /// O codegen emite `jump epilogue_block(value)` — o epílogo faz
    /// `arena_destroy(local_arena)` + `return_(result)`.
    /// Statements após `return` são unreachable (não produzidos pelo typeck).
    Return(Box<Spanned<TypedExpr>>),

    // ── Fio 3 Fase 4: loop, break, continue ──────────────────────
    /// `loop` — laço infinito. Body é uma sequência de expressões tipadas.
    /// O tipo do loop é determinado pelo tipo do `break` (ou Unit se sem break).
    Loop { body: Vec<Spanned<TypedExpr>> },

    /// `break` — sai do laço.
    /// O tipo do `break` determina o tipo do loop (unificado entre todos breaks).
    Break,

    /// `continue` — próxima iteração.
    Continue,
}

/// Informação sobre uma variável capturada por uma closure.
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    pub name: String,
    pub ty: Ty,
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

/// Pattern tipado — pattern da AST com tipo resolvido para bindings.
///
/// O typeck resolve `Pattern::Ident("True")` para `TypedPattern::Variant`
/// se `True` é variante do enum do scrutinee. Para `Ident("x")` que não é
/// variante, o typeck mantém `Ident` e liga `x` ao tipo do scrutinee.
#[derive(Debug, Clone)]
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

/// Módulo tipado — artefato final do Pass 2.
///
/// Contém a TAST do entry point, o DispatchTable populado com todas as
/// assinaturas (prelude + módulo do usuário), e as funções nomeadas com
/// corpo Kata (já tipadas). O codegen consome isto.
#[derive(Debug, Clone)]
pub struct TypedModule {
    /// Expressões top-level anteriores ao entry point (let bindings, etc.).
    /// Loweradas em sequência antes do entry — compartilham o var_map.
    pub pre_entry: Vec<Spanned<TypedExpr>>,
    /// Entry point tipado — última expressão top-level do módulo.
    pub entry: Spanned<TypedExpr>,
    /// DispatchTable populado com prelude + assinaturas do módulo.
    pub dispatch_table: DispatchTable,
    /// Snapshot do TypeEnv ao final do typeck (para inspeção/debug).
    pub type_env: TypeEnv,
    /// Funções nomeadas com corpo Kata (não-FFI), já tipadas.
    /// Cada função vira uma função Cranelift separada no codegen.
    pub functions: Vec<TypedFunction>,
    /// Actions tipadas (Fio 3). Cada Action vira uma função Cranelift
    /// com ABI estendido (caller_arena handle como primeiro param).
    pub actions: Vec<TypedAction>,
}

/// Função nomeada tipada — pronta para o codegen.
///
/// Produzida pelo inference quando `FunctionDef` (do resolution) tem cláusulas.
/// O codegen declara uma função Cranelift com `name`, assinatura
/// `(param_types → ret_ty)`, e lowera cada cláusula como branch chain.
#[derive(Debug, Clone)]
pub struct TypedFunction {
    /// Nome da função no JITModule (para call direto).
    pub name: String,
    /// Tipos dos parâmetros (da assinatura do Sig).
    pub param_types: Vec<Ty>,
    /// Tipo de retorno (da assinatura do Sig).
    pub ret_ty: Ty,
    /// Cláusulas tipadas (padrões + corpo + guards + with bindings).
    pub clauses: Vec<TypedLambdaClause>,
}

/// Action tipada — pronta para o codegen (Fio 3).
///
/// Produzida pelo inference quando `ActionDef` (do resolution) é encontrado.
/// O codegen declara uma função Cranelift com ABI estendido:
/// `(caller_arena: i64, arg1, ...) -> ret_ty`.
#[derive(Debug, Clone)]
pub struct TypedAction {
    /// Nome da Action no JITModule.
    pub name: String,
    /// Tipos dos parâmetros (elementos da tupla de argumentos).
    pub param_types: Vec<Ty>,
    /// Tipo de retorno.
    pub ret_ty: Ty,
    /// Body da Action (statements sequenciais).
    pub body: Vec<Spanned<TypedExpr>>,
}
