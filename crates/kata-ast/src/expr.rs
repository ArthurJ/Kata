//! `Expr` — expressões da AST (saída do parser, entrada do typeck).
//!
//! A AST é plana e sem lógica — apenas dados. O typeck produz a TAST
//! (TypedExpr) a partir destes nós.

use crate::span::Spanned;

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
    /// `3.14::Rational` (rebaixa FloatLit a Rational).
    /// `5::PositiveInt` (valida predicados, entrega refined).
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
    /// `let (x, y, ...) := expr` — destructuring de tupla.
    /// O typeck desugaring: `let __t := expr; let x := __t.0; let y := __t.1; ...`
    /// Os nomes são os bindings; `_` pula o elemento (não gera binding).
    LetDestruct {
        names: Vec<String>,
        value: Box<Spanned<Expr>>,
    },

    // ── Qualificação de variante ────────────────────────
    /// `Enum::Variante` — qualificação de variante de enum.
    /// `Boolean::True`, `Result::Ok`, etc.
    /// O parser não sabe se `Boolean` é tipo ou módulo — produz
    /// `VariantQual` e o typeck resolve.
    ///
    /// `module_path` é `Some` quando a sintaxe é qualificada por módulo:
    /// `core.Result::Err` → `module_path = Some(["core"])`,
    /// `enum_name = "Result"`, `variant = "Err"`.
    /// Quando `None`, é a forma não-qualificada `Result::Err`.
    VariantQual {
        enum_name: String,
        variant: String,
        module_path: Option<Vec<String>>,
    },

    // ── Funções, Lambdas, Match, Hole, Pipe ─────────────
    /// `lambda <padrões>: <corpo>` — lambda anônimo (cláusula única).
    ///
    /// Se `guards` é vazio: `body` é a expressão única após `:`.
    ///   `lambda x: + x 1`
    ///
    /// Se `guards` é não-vazio: o corpo é um bloco indentado de guard clauses.
    ///   `lambda x:`
    ///       `> x 0: x`
    ///       `otherwise: - 0 x`
    ///   Neste caso, `body` é ignorado (ou pode ser usado como fallback final).
    Lambda {
        patterns: Vec<Spanned<Pattern>>,
        body: Box<Spanned<Expr>>,
        /// Guards opcionais dentro do corpo. Se não-vazio, o corpo é
        /// uma sequência de guard clauses (bloco indentado após `:`).
        guards: Vec<GuardClause>,
        /// `with` block opcional (bindings prévios, pós-escritos mas pré-avaliados).
        with_bindings: Vec<WithBinding>,
    },

    /// `match <scrutinee>` com braços indentados.
    Match {
        scrutinee: Box<Spanned<Expr>>,
        arms: Vec<MatchArm>,
    },

    /// `_` em posição de argumento — hole para currying.
    /// O parser produz `Expr::Hole` quando encontra `Ident("_")` em posição
    /// de argumento de `Apply`. Em posição de pattern, o parser produz
    /// `Pattern::Wildcard` — a disambiguação é no parser, não no typeck.
    /// Desugared pelo typeck em Lambda. Nunca chega à TAST.
    Hole,

    /// `lhs |> rhs` — pipeline.
    /// Desugared pelo typeck. Nunca chega à TAST.
    Pipe {
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },

    // ── Actions, return, var, loop, break, continue ───────────
    /// `nome!(args)` — chamada de Action.
    /// `!` é o marcador de impureza. O parser produz `ActionCall` quando vê `!`
    /// após um identificador seguido de parênteses (tupla de argumentos).
    ActionCall {
        callee: String,
        /// Tupla de argumentos (sempre uma tupla, mesmo que vazia: `!()`).
        args: Box<Spanned<Expr>>,
    },

    /// `type!(expr)` — introspecção compile-time.
    /// Retorna o tipo nominal de `expr` como `Text`. O typeck resolve
    /// o tipo em compile-time; o monomorphizador substitui por `TextLit`.
    /// `type` é keyword do lexer — o parser reconhece `Type` seguido de `!`.
    TypeOf { expr: Box<Spanned<Expr>> },

    /// `return expr` — early return em Actions.
    /// Exclusivo de Actions. Não existe em funções puras.
    Return(Box<Spanned<Expr>>),

    /// `loop` — laço infinito. Só sai via `break`.
    /// Body é uma sequência de statements (expressões).
    Loop { body: Vec<Spanned<Expr>> },

    /// `break` — sai do laço.
    Break,

    /// `continue` — próxima iteração.
    Continue,

    /// `var nome := expr` — binding mutável (exclusivo de Actions).
    Var {
        name: String,
        value: Box<Spanned<Expr>>,
    },

    /// `nome := expr` — reatribuição a variável `var` (exclusivo de Actions).
    /// O parser produz `Reassign` quando vê `Ident :=` sem `let`/`var` prefix.
    Reassign {
        name: String,
        value: Box<Spanned<Expr>>,
    },

    /// `expr ?` — fail-fast (exclusivo de Actions).
    /// Desugared pelo typeck em Match + Return. Nunca chega à TAST.
    Question(Box<Spanned<Expr>>),

    /// `lhs | rhs` — fallback local (coalescência de erro).
    /// Desugared pelo typeck em Match. Nunca chega à TAST.
    /// Distinto de `|>` (PipeForward — pipeline de transformação pura).
    PipeFallback {
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },

    // ── DotAccess (field access + index access) ──────────
    /// `expr.nome` ou `expr.0` — access unificado.
    /// O parser não decide se é field ou index — o typeck resolve
    /// pelo tipo do receptor (`Struct` → field, `Tuple` → index).
    DotAccess {
        expr: Box<Spanned<Expr>>,
        index: DotIndex,
    },

    /// `$` — marcador de spread (typeck expande, nunca chega à TAST).
    Spread,

    // ── Coleções ────────────────────────────────────
    /// `[1 2 3]` — lista literal (Cons cells).
    ListLit { elements: Vec<Spanned<Expr>> },

    /// `{1 2 3}` — array literal (contíguo, imutável).
    ArrayLit { elements: Vec<Spanned<Expr>> },

    /// `{"k": v "k2": v2}` — literal de Dict.
    DictLit {
        entries: Vec<(Spanned<Expr>, Spanned<Expr>)>,
    },
    /// `{|1 2 3|}` — literal de Set.
    SetLit { elements: Vec<Spanned<Expr>> },

    /// `[a..s..b]` ou `[a..s..=b]` — range lazy.
    /// Step é sempre explícito na sintaxe.
    RangeLit {
        start: Box<Spanned<Expr>>,
        step: Box<Spanned<Expr>>,
        end: Box<Spanned<Expr>>,
        /// true = `..=` (inclusive), false = `..` (exclusive)
        inclusive: bool,
    },

    /// `for x in colecao` — iteração via ITERABLE (exclusivo de Actions).
    ForIn {
        var_name: String,
        iterable: Box<Spanned<Expr>>,
        body: Vec<Spanned<Expr>>,
    },

    /// `x in coll` — operador de membership (dispatch via CONTAINS).
    In {
        item: Box<Spanned<Expr>>,
        collection: Box<Spanned<Expr>>,
    },

    // ── CSP (Canais, Fork, Select) ───────────────────
    /// `tx !> valor` — envio por canal.
    ChannelSend {
        channel: Box<Spanned<Expr>>,
        value: Box<Spanned<Expr>>,
    },

    /// `rx <! nome` — recebimento de canal (binding em `nome`).
    ChannelRecv {
        channel: Box<Spanned<Expr>>,
        bind_name: String,
    },

    /// `select` com braços de canal e timeout.
    Select {
        arms: Vec<SelectArm>,
        timeout_ms: Option<Box<Spanned<Expr>>>,
        timeout_body: Option<Box<Spanned<Expr>>>,
    },
}

/// Índice de DotAccess — field nomeado ou inteiro.
#[derive(Debug, Clone, PartialEq)]
pub enum DotIndex {
    /// `expr.nome` — field access em struct.
    Field(String),
    /// `expr.0`, `expr.(-1)` — index access em tupla.
    /// Negativos são resolvidos em compile-time (`-1` = `len-1`).
    Int(i64),
}

/// Pattern — usado em match arms e cláusulas lambda.
///
/// Disambiguação no parser: `_` em posição de pattern → `Wildcard`.
/// `True` em posição de pattern → `Ident("True")` (typeck resolve via
/// `EnumRegistry` para `Variant` se for variante de enum do scrutinee).
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `x` — liga o valor ao nome.
    Ident(String),
    /// `_` — wildcard, aceita qualquer valor sem ligar nome.
    Wildcard,
    /// `42`, `"texto"`, `3.14` — literal exato.
    Literal(Spanned<Expr>),
    /// `Boolean::True`, `Result::Ok` — variante de enum qualificada.
    /// `payload` é None para variantes unitárias (`True`, `None`).
    /// `Some(vec![sub_pat])` para variantes com payload (`Ok(v)`, `Some(x)`).
    Variant {
        enum_name: String,
        variant: String,
        /// Sub-patterns do payload. None = unitária.
        /// Some(vec) = variantes com payload (1 elemento por enquanto).
        payload: Option<Vec<Spanned<Pattern>>>,
    },
    /// `(a, b, c)` — tupla.
    Tuple(Vec<Spanned<Pattern>>),
    /// `[h : t]` — cons (cabeça : cauda).
    /// Reconhece a sintaxe; List dá semântica de runtime.
    Cons {
        head: Box<Spanned<Pattern>>,
        tail: Box<Spanned<Pattern>>,
    },
    /// `[]` — lista vazia (Nil). Testa `val == 0` no codegen.
    Nil,
}

/// Uma cláusula lambda após uma assinatura.
/// Múltiplas cláusulas = função nomeada; 1 cláusula = lambda anônimo.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaClause {
    pub patterns: Vec<Spanned<Pattern>>,
    pub body: Spanned<Expr>,
    pub guards: Vec<GuardClause>,
    pub with_bindings: Vec<WithBinding>,
}

/// Um guard: `condição: corpo` ou `otherwise: corpo`.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardClause {
    /// `None` = `otherwise` (sempre passa).
    pub condition: Option<Spanned<Expr>>,
    pub body: Spanned<Expr>,
}

/// Um braço de match: `pattern: corpo` ou `otherwise: corpo`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    /// `None` = `otherwise` (fallback).
    pub pattern: Option<Spanned<Pattern>>,
    /// Guard opcional após pattern ( : não implementado no parser ainda).
    pub guard: Option<Spanned<Expr>>,
    pub body: Spanned<Expr>,
}

/// Um braço de `select`: `rx <! nome: body`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectArm {
    /// Receiver de onde receber (expressão que avalia para Receiver::T).
    pub channel: Spanned<Expr>,
    /// Nome do binding para o valor recebido.
    pub bind_name: String,
    /// Corpo do braço.
    pub body: Spanned<Expr>,
}

/// Binding de `with` block: `nome := expr` (sem keyword `let`).
#[derive(Debug, Clone, PartialEq)]
pub struct WithBinding {
    pub name: String,
    pub value: Spanned<Expr>,
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
    /// Não usado (enums genéricos).
    ParamApp {
        name: String,
        params: Vec<Spanned<TypeExpr>>,
    },

    /// `(T1, T2, ...)` — tipo tupla. Múltiplos tipos separados por vírgula.
    Tuple(Vec<Spanned<TypeExpr>>),

    /// `(T1 -> T2)` — tipo de função como valor.
    /// Exige parênteses para desambiguar.
    Func {
        params: Vec<Spanned<TypeExpr>>,
        ret: Box<Spanned<TypeExpr>>,
    },

    /// `Action(Param1, Param2, ...) => Ret` — tipo de Action first-class.
    /// Espelha a assinatura de actions, sem nomes dos params.
    ActionType {
        params: Vec<Spanned<TypeExpr>>,
        ret: Box<Spanned<TypeExpr>>,
    },

    /// `Self` — referência ao tipo que implementa a interface.
    /// Válido apenas dentro de blocos `interface` e `implements`.
    /// O resolution substitui pelo tipo concreto no impl.
    SelfRef,

    /// `T?` — açúcar sintático para `Result::(T, Err)`.
    /// O `?` em posição de tipo é parsed como postfix após qualquer
    /// TypeExpr base. O resolution desaçuca para
    /// `Ty::Generic("Result", [resolve(T), Ty::Text])`.
    /// Distinto do `?` em posição de expressão (operador fail-fast,
    /// exclusivo de Actions — `Expr::Question`).
    Question(Box<Spanned<TypeExpr>>),

    /// `module.Type` — tipo qualificado de módulo importado.
    /// O resolution procura no TypeEnv por um binding onde
    /// `name == name && origin == module`.
    Qualified { module: String, name: String },
}

/// Statement do body de uma Action.
///
/// Cada statement é uma expressão com uma marca de `;` — `has_semicolon = true`
/// significa computação local (valor descartado); `has_semicolon = false` no
/// último statement significa retorno implícito (valor é retornado).
#[derive(Debug, Clone, PartialEq)]
pub struct ActionStmt {
    pub expr: Spanned<Expr>,
    pub has_semicolon: bool,
}

/// Um módulo completo — arquivo .kata.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub items: Vec<Spanned<crate::item::Item>>,
}

impl Module {
    pub fn new(items: Vec<Spanned<crate::item::Item>>) -> Self {
        Module { items }
    }
}
