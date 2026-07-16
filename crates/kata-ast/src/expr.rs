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

    // ── Fio 2: Funções, Lambdas, Match, Hole, Pipe ─────────────
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

    // ── Fio 3: Actions, return, var, loop, break, continue ───────────
    /// `nome!(args)` — chamada de Action.
    /// `!` é o marcador de impureza. O parser produz `ActionCall` quando vê `!`
    /// após um identificador seguido de parênteses (tupla de argumentos).
    ActionCall {
        callee: String,
        /// Tupla de argumentos (sempre uma tupla, mesmo que vazia: `!()`).
        args: Box<Spanned<Expr>>,
    },

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

    // ── Fio 5: DotAccess (field access + index access) ──────────
    /// `expr.nome` ou `expr.0` — access unificado.
    /// O parser não decide se é field ou index — o typeck resolve
    /// pelo tipo do receptor (`Struct` → field, `Tuple` → index).
    DotAccess {
        expr: Box<Spanned<Expr>>,
        index: DotIndex,
    },

    /// `$` — marcador de spread (typeck expande, nunca chega à TAST).
    Spread,

    // ── Fio 8: Coleções ────────────────────────────────────
    /// `[1 2 3]` — lista literal (Cons cells).
    ListLit { elements: Vec<Spanned<Expr>> },

    /// `{1 2 3}` — array literal (contíguo, imutável).
    ArrayLit { elements: Vec<Spanned<Expr>> },

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
    /// `[h : t]` — cons (cabeça : cauda). `[]` para lista vazia.
    /// Fio 2 reconhece a sintaxe; Fio 8 (List) dá semântica de runtime.
    /// Em Fio 2, pattern Cons/Nil só funciona se List existir (não existe
    /// ainda — stub que produz erro limpo).
    Cons {
        head: Box<Spanned<Pattern>>,
        tail: Box<Spanned<Pattern>>,
    },
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
    /// Guard opcional após pattern (Fio 2: não implementado no parser ainda).
    pub guard: Option<Spanned<Expr>>,
    pub body: Spanned<Expr>,
}

/// Binding de `with` block: `nome := expr` (sem keyword `let`).
#[derive(Debug, Clone, PartialEq)]
pub struct WithBinding {
    pub name: String,
    pub value: Spanned<Expr>,
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
        // Fio 1: sempre None (FFI — corpo suprido por @ffi).
        // Fio 2: Some(clauses) = função pura com corpo Kata.
        body: Option<Vec<Spanned<LambdaClause>>>,
    },

    // ── Declarações de tipo ─────────────────────────────
    /// `data Nome ()` — tipo opaco (sem campos).
    /// Em Fio 1: `data Int ()` com `@ffi("i64")`.
    /// Fio 5 trará campos: `data Pessoa (nome::Text idade::Int)`.
    /// Fio 6 trará refined: `data (Int, > _ 0) as PositiveInt`.
    DataDecl {
        name: String,
        fields: Vec<FieldDecl>, // vazio para tipos opacos de Fio 1
        directives: Vec<Directive>,
        /// Fio 6: refined declaration. None = struct normal.
        /// Some(RefinedDecl) = tipo refinado com predicados.
        refined: Option<RefinedDecl>,
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

    // ── Fio 5: Aliases ──────────────────────────────────
    /// `alias Target as NewName` — cria um newtype (tipo nominal distinto
    /// com o mesmo layout do target). O construtor sintetizado é identity:
    /// `NewName :: Target => NewName`.
    AliasDecl { target: String, new_name: String },

    // ── Fio 3: Actions ─────────────────────────────────
    /// `action nome` com body indentado.
    /// Actions são o domínio impuro. Declaração com `action nome` (sem `!`).
    /// O body é uma sequência de statements (`ActionStmt`).
    ActionDecl {
        name: String,
        /// Parâmetros da Action (uma tupla tipada, ou vazia).
        params: Vec<Spanned<TypeExpr>>,
        ret: Spanned<TypeExpr>,
        directives: Vec<Directive>,
        /// Body da Action (statements sequenciais).
        body: Vec<ActionStmt>,
    },

    // ── Fio 7: Interfaces, Implements, Import, Export ──────────
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
        /// Type params do tipo (ex: `A` em `List(A) implements ITERABLE(A)`).
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
/// Fio 6: o conteúdo de `()` é um TypeExpr (base) seguido de predicados (Expr).
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
    /// Fio 6: predicado da variante. None = sem predicado.
    /// `Magreza(< _ 18.5)` → predicate = Some(Apply { <, [Hole, 18.5] }).
    /// `Obesidade` → predicate = None (default/fallback).
    pub predicate: Option<Spanned<Expr>>,
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

    /// `(T1, T2, ...)` — tipo tupla. Múltiplos tipos separados por vírgula.
    Tuple(Vec<Spanned<TypeExpr>>),

    /// `(T1 -> T2)` — tipo de função como valor.
    /// Exige parênteses para desambiguar.
    Func {
        params: Vec<Spanned<TypeExpr>>,
        ret: Box<Spanned<TypeExpr>>,
    },

    /// `Self` — referência ao tipo que implementa a interface.
    /// Válido apenas dentro de blocos `interface` e `implements`.
    /// O resolution substitui pelo tipo concreto no impl.
    SelfRef,
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
    pub items: Vec<Spanned<Item>>,
}

impl Module {
    pub fn new(items: Vec<Spanned<Item>>) -> Self {
        Module { items }
    }
}

// ── Fio 7: Structs auxiliares para interfaces ──────────────

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
