//! `TypedExpr` — TAST enriquecida (saída do typeck, entrada do codegen).
//!
//! Cada nó carrega `ty: Ty`, `tail_pos: bool`, `effect: Effect` — os três
//! populados desde o início para evitar retrofit (PRD-fio1 §17, Risco 5).
//!
//! `TypedExprKind` espelha `Expr` mas com `Spanned<TypedExpr>` em vez de
//! `Spanned<Expr>` — a recursão é sobre a TAST, não sobre a AST.

use kata_ast::{Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::Ty;

// Tipos de matching (patterns, cláusulas, guards, with bindings) foram
// extraídos para `typed_pattern.rs` — ver [`crate::typed_pattern`].
pub use crate::typed_pattern::{
    TypedGuardClause, TypedLambdaClause, TypedMatchArm, TypedPattern, TypedWithBinding,
};

/// Efeito de uma expressão. só produz `Puro`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Expressão pura — sem efeitos colaterais.
    Puro,
    /// Efeito de I/O — `echo`, leitura/escrita.
    IO,
    /// Spawn de fiber — `spawn`.
    Spawn,
    /// Operação de canal — `send`/`recv`.
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
    /// tail calls (fios posteriores) e o TRMA pass (fios posteriores).
    pub tail_pos: bool,
    /// Destino de escape para seleção de arena (Pré-11).
    /// Generaliza `tail_pos` para memória: `Local` = fiber_arena,
    /// `Caller` = caller_arena.
    /// Coexiste com `tail_pos` (que governa TCO, não memória).
    pub escape: EscapeTarget,
    /// Efeito da expressão. `Puro`.
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
    /// Chamada de função na TAST. Renomeado de `Apply` (PRD-fio2).
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
    /// Tupla heterogênea. Aceita para patterns (Ty::Tuple antecipado).
    Tuple { elements: Vec<Spanned<TypedExpr>> },
    /// Construção de struct — smart constructor body.
    /// Aloca `n * 8` bytes na arena e faz `store` por campo.
    /// `struct_name` é a identidade nominal (ex: `Pessoa`).
    /// `values` é um `Spanned<TypedExpr>` por campo, em ordem de declaração.
    /// Semanticamente idêntico a `Tuple` no layout — só muda identidade nominal.
    StructConstruct {
        struct_name: String,
        values: Vec<Spanned<TypedExpr>>,
    },
    /// `expr.nome` — field access em struct.
    /// `field_index` é o offset em words (já calculado pelo typeck via StructRegistry).
    /// Codegen faz `load ptr + field_index * 8`.
    FieldAccess {
        expr: Box<Spanned<TypedExpr>>,
        struct_name: String,
        field_name: String,
        field_index: u32,
    },
    /// `expr.N` — index access em tupla.
    /// `element_index` é o offset em words (já resolvido, negativos normalizados).
    /// Codegen faz `load ptr + element_index * 8`.
    IndexAccess {
        expr: Box<Spanned<TypedExpr>>,
        index: i64,
        element_index: u32,
    },
    /// `let nome := expr` — binding imutável. O typeck definiu `nome` no
    /// TypeEnv com `ty = expr.ty`. `tail_pos = false` sempre.
    Let {
        name: String,
        value: Box<Spanned<TypedExpr>>,
    },
    /// `let (x, y, ...) := expr` — destructuring de tupla.
    /// `temp_name` recebe o valor; cada binding em `bindings` é
    /// `FieldAccess(temp_name, i)` com o tipo do elemento.
    /// O codegen faz `def_var(temp_name, value)` seguido de
    /// `def_var(name, field_access)` para cada binding.
    LetDestruct {
        temp_name: String,
        value: Box<Spanned<TypedExpr>>,
        bindings: Vec<(String, Spanned<TypedExpr>)>,
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
    /// Sum com payload.
    /// `tag` é o índice da variante no enum (para codegen).
    VariantConstruct {
        enum_name: String,
        variant: String,
        payload: Box<Spanned<TypedExpr>>,
        tag: usize,
    },

    // ── Lambda, Match ──────────────────────────────────────
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
        /// Variáveis capturadas do escopo externo.
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

    // ── Actions, var ──────────────────────────────────────
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
        /// None para Actions definadas pelo usuário (despacha via kata_refs).
        ffi_symbol: Option<String>,
        /// None = call direto (lookup em kata_refs pelo nome).
        /// Some(expr) = call indireto (fn_ptr vem da expressão — variável/param).
        indirect_callee: Option<Box<Spanned<TypedExpr>>>,
    },

    /// `type!(expr)` — introspecção compile-time.
    /// O typeck atribui `ty = Text`. O monomorphizador substitui
    /// por avaliação de `expr` (preserva side-effects) + `TextLit(ty_display(&expr.ty))`.
    TypeOf { expr: Box<Spanned<TypedExpr>> },

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

    // ── Loop, break, continue ──────────────────────
    /// `loop` — laço infinito. Body é uma sequência de expressões tipadas.
    /// O tipo do loop é determinado pelo tipo do `break` (ou Unit se sem break).
    Loop { body: Vec<Spanned<TypedExpr>> },

    /// `break` — sai do laço.
    /// O tipo do `break` determina o tipo do loop (unificado entre todos breaks).
    Break,

    /// `continue` — próxima iteração.
    Continue,

    // ── Coleções ──────────────────────────────────────────
    /// `[1 2 3]` — lista literal (Cons cells). `elem_ty` é o tipo unificado.
    ListLit { elements: Vec<Spanned<TypedExpr>> },

    /// `{1 2 3}` — array literal (contíguo).
    ArrayLit { elements: Vec<Spanned<TypedExpr>> },

    /// `{"k": v ...}` — literal de Dict.
    DictLit {
        entries: Vec<(Spanned<TypedExpr>, Spanned<TypedExpr>)>,
        key_ty: Ty,
        value_ty: Ty,
    },
    /// `{|1 2 3|}` — literal de Set.
    SetLit {
        elements: Vec<Spanned<TypedExpr>>,
        elem_ty: Ty,
    },

    /// `[a..s..b]` ou `[a..s..=b]` — range lazy.
    /// `elem_ty` é o tipo do elemento (start/step/end mesmo tipo A).
    /// `inclusive` = true para `..=`, false para `..`.
    RangeLit {
        start: Box<Spanned<TypedExpr>>,
        step: Box<Spanned<TypedExpr>>,
        end: Box<Spanned<TypedExpr>>,
        inclusive: bool,
        elem_ty: Ty,
    },

    /// `for x in colecao` — iteração via ITERABLE.
    /// `var_ty` é o tipo A extraído do InterfaceRegistry.
    /// O tipo do ForIn é Unit (como `loop`).
    ForIn {
        var_name: String,
        var_ty: Ty,
        iterable: Box<Spanned<TypedExpr>>,
        body: Vec<Spanned<TypedExpr>>,
    },

    /// `x in coll` — membership via CONTAINS. Tipo: Boolean.
    In {
        item: Box<Spanned<TypedExpr>>,
        collection: Box<Spanned<TypedExpr>>,
    },

    // ── Higher-order — map/filter/fold ──────────
    /// `map f coll` — aplica f a cada elemento, retorna List(B).
    /// coll_ty é o tipo concreto (List/Array/Range). ret_ty é List(B).
    /// Se coll_ty é Array, o codegen converte List→Array no final.
    Map {
        callback: Box<Spanned<TypedExpr>>,
        collection: Box<Spanned<TypedExpr>>,
        coll_ty: Ty,
        elem_ty: Ty,
        ret_ty: Ty,
    },

    /// `filter f coll` — filtra elementos por predicado, retorna List(A).
    Filter {
        callback: Box<Spanned<TypedExpr>>,
        collection: Box<Spanned<TypedExpr>>,
        coll_ty: Ty,
        elem_ty: Ty,
        ret_ty: Ty,
    },

    /// `fold f init coll` — reduz coleção com função e acumulador.
    /// ret_ty é o tipo do acumulador (init).
    Fold {
        callback: Box<Spanned<TypedExpr>>,
        initial: Box<Spanned<TypedExpr>>,
        collection: Box<Spanned<TypedExpr>>,
        coll_ty: Ty,
        elem_ty: Ty,
        ret_ty: Ty,
    },

    /// `map f (filter g coll)` (e composições) — stream fusion (DoD 60).
    /// Itera a coleção fonte aplicando os estágios em cadeia num único loop,
    /// sem materializar listas intermediárias.
    ///
    /// `stages` é uma cadeia de transformações: Filter aplica o predicado
    /// (descarta elemento se false), Map aplica a transformação.
    /// `source_elem_ty` é o tipo do elemento da coleção fonte.
    /// `result_elem_ty` é o tipo do elemento após todos os estágios.
    /// `ret_ty` é sempre `List(result_elem_ty)`.
    FusedStream {
        stages: Vec<FusedStage>,
        source: Box<Spanned<TypedExpr>>,
        coll_ty: Ty,
        source_elem_ty: Ty,
        result_elem_ty: Ty,
        ret_ty: Ty,
    },

    // ── CSP — canais, select, fork ──────────────────────
    /// `tx !> valor` — envio por canal (effect = ChannelOp).
    ChannelSend {
        channel: Box<Spanned<TypedExpr>>,
        value: Box<Spanned<TypedExpr>>,
    },

    /// `rx <! nome` — recebimento de canal (effect = ChannelOp).
    /// `recv_ty` é o tipo do valor recebido (inferido do tipo do canal).
    ChannelRecv {
        channel: Box<Spanned<TypedExpr>>,
        recv_ty: Ty,
        bind_name: String,
    },

    /// `select` com braços (effect = ChannelOp).
    Select {
        arms: Vec<TypedSelectArm>,
        timeout_ms: Option<Box<Spanned<TypedExpr>>>,
        timeout_body: Option<Box<Spanned<TypedExpr>>>,
    },

    /// `channel!()`, `queue!(N)`, `broadcast!()` — criação de canal.
    /// Interceptado em `infer_apply` (não despacha para DispatchTable).
    /// Retorna `(Sender::T, Receiver::T)` ou `(Sender::T, ReceiverFactory::T)`.
    ChannelCreate {
        /// Tipo de canal: rendezvous, bufferizado, ou broadcast.
        kind: ChannelKind,
        /// Tipo do valor transportado (`T` em `Sender::T`).
        elem_ty: Ty,
    },

    /// `rxf!()` — pedido de receiver a uma ReceiverFactory existente.
    /// O `factory` avalia para um handle de `ReceiverFactory::T`; o codegen
    /// chama `kata_rt_broadcast_receiver_create(arena, factory_handle)` e
    /// retorna um `Receiver::T` independente (latest-only, future-only).
    ReceiverFactoryCall {
        /// Expressão que avalia para o handle da ReceiverFactory.
        factory: Box<Spanned<TypedExpr>>,
        /// Tipo do valor transportado (`T` em `Receiver::T`).
        elem_ty: Ty,
    },

    /// `fork!(action, args)` — spawn de fiber (effect = Spawn).
    /// `action_name` é o nome da Action a executar no novo fiber.
    /// `action_expr` é a expressão tipada que avalia para o fn_ptr da Action.
    /// Para Ident direto (worker), é o Ident. Para variável (f), é a variável.
    /// `args` é a tupla de argumentos tipados.
    Fork {
        action_name: String,
        /// Expressão que avalia para o fn_ptr da Action.
        /// Para Ident direto (worker), é o Ident. Para variável (f), é a variável.
        action_expr: Box<Spanned<TypedExpr>>,
        args: Box<Spanned<TypedExpr>>,
    },
}

/// Tipo de canal criado por `channel!()`, `queue!(N)`, `broadcast!()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelKind {
    /// `channel!()` — rendezvous (síncrono, sem buffer).
    Rendezvous,
    /// `queue!(N)` — bufferizado com capacidade N.
    Buffered(i64),
    /// `broadcast!()` — pub-sub, latest only.
    Broadcast,
}

/// Braço de `select` na TAST.
#[derive(Debug, Clone)]
pub struct TypedSelectArm {
    pub channel: Spanned<TypedExpr>,
    pub recv_ty: Ty,
    pub bind_name: String,
    pub body: Spanned<TypedExpr>,
}

/// Um estágio de um `FusedStream` — transformação individual na cadeia.
#[derive(Debug, Clone)]
pub enum FusedStage {
    /// `filter g` — descarta elemento se predicado retorna false.
    Filter {
        callback: Box<Spanned<TypedExpr>>,
        /// Tipo do elemento que este estágio recebe.
        input_elem_ty: Ty,
    },
    /// `map f` — transforma elemento.
    Map {
        callback: Box<Spanned<TypedExpr>>,
        /// Tipo do elemento que este estágio recebe.
        input_elem_ty: Ty,
        /// Tipo do elemento que este estágio produz.
        output_elem_ty: Ty,
    },
}

/// Informação sobre uma variável capturada por uma closure.
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    pub name: String,
    pub ty: Ty,
}

// Artefatos tipados de nível de módulo (TypedModule, TypedFunction,
// TypedAction, TypedTestSpec, TypedLogSpec) foram extraídos para
// `typed_module.rs` — ver [`crate::typed_module`].
pub use crate::typed_module::{
    TypedAction, TypedFunction, TypedLogSpec, TypedModule, TypedTestSpec,
};
