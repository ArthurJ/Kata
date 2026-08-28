//! Artefatos tipados de nível de módulo — `TypedModule`, `TypedFunction`,
//! `TypedAction`, `TypedTestSpec`.
//!
//! Saída final do Pass 2 (inference) no que tange ao agrupamento por módulo:
//! o entry point tipado, as funções nomeadas e as actions. O codegen e o
//! tree shaking consomem estes tipos. A TAST em si (nó de expressão
//! `TypedExpr`/`TypedExprKind` e auxiliares de CSP) vive em
//! [`crate::typed`].

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;
use kata_core::snapshot::HeapSnapshotData;
use kata_core::ty::{Ty, TypeEnv};
use kata_resolution::RefinedDeclInfo;
pub use kata_resolution::TimerSpec;

use crate::typed::TypedExpr;
use crate::typed_pattern::TypedLambdaClause;

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
    /// Actions tipadas. Cada Action vira uma função Cranelift
    /// com ABI estendido (caller_arena handle como primeiro param).
    pub actions: Vec<TypedAction>,
    /// Catálogo de structs com alias_of/predicates — para o codegen
    /// resolver o Cranelift type correto de refined/alias de primitivos.
    pub struct_registry: kata_core::StructRegistry,
    /// Snapshots de valores comptime — bytes serializados para embed no binário.
    /// O comptime pass popula esta tabela; o codegen emite como dados estáticos;
    /// o runtime faz `kata_rt_load_snapshots` em load-time.
    pub snapshots: Vec<HeapSnapshotData>,
    /// Declarações de tipos refinados — predicados para validação deferred.
    /// O typeck valida predicados triviais localmente; predicados complexos
    /// (que envolvem chamada de função) são delegados ao comptime pass, que
    /// tem acesso a `jit_eval`. Populado por `infer_module`.
    pub refined_decls: Vec<RefinedDeclInfo>,
    /// Constantes de módulo — `constant nome := expr`. O comptime pass avalia
    /// cada `value` via JIT-and-execute e substitui por literal/HeapSnapshot.
    /// O codegen lowera no prólogo de `__kata_entry`. Acessível de actions,
    /// funções nomeadas, lambdas, e entry point.
    pub constants: Vec<Spanned<TypedExpr>>,
}

/// Estratégia de eviction do cache `@cache`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStrategy {
    LRU,
    FIFO,
    MRU,
    LFU,
}

/// Especificação de cache `@cache{strategy: "LRU", capacity: 256}`.
///
/// Anota uma função para memoização. O codegen emite cache lookup no prólogo
/// e insert no epílogo. Cache lazy-allocated em TLS (futuramente em
/// `caller_arena`).
#[derive(Debug, Clone, PartialEq)]
pub struct CacheSpec {
    /// Estratégia de eviction.
    pub strategy: CacheStrategy,
    /// Número máximo de entradas antes de eviction.
    pub capacity: i64,
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
    /// Especificação de cache `@cache`. None se a função não tem `@cache`.
    pub cache_spec: Option<CacheSpec>,
    /// Especificação de timer `@timer`. None se a função não tem `@timer`.
    pub timer_spec: Option<TimerSpec>,
}

/// Action tipada — pronta para o codegen.
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
    /// Nomes dos params. `Some(nome)` para params nomeados (`x::Tipo`),
    /// `None` para posicional legado. Paralelo a `param_types`.
    pub param_names: Vec<Option<String>>,
    /// Tipo de retorno.
    pub ret_ty: Ty,
    /// Body da Action (statements sequenciais).
    pub body: Vec<Spanned<TypedExpr>>,
    /// Casos de teste `@test` com args já tipados. O codegen gera
    /// um wrapper por spec.
    pub tests: Vec<TypedTestSpec>,
}

/// `TestSpec` tipado — args já inferidos pelo typeck.
///
/// Produzido pelo inference a partir do `TestSpec` do resolution:
/// `args: Option<Spanned<Expr>>` → `args: Option<Spanned<TypedExpr>>`.
/// O codegen lê `args` para lowerar a tupla de argumentos do wrapper.
#[derive(Debug, Clone)]
pub struct TypedTestSpec {
    pub desc: Option<String>,
    pub args: Option<Spanned<TypedExpr>>,
    pub timeout: Option<i64>,
    pub expects: Option<String>,
    pub policy: Option<kata_resolution::MatchPolicy>,
}
