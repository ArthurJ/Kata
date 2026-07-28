//! Artefatos tipados de nível de módulo — `TypedModule`, `TypedFunction`,
//! `TypedAction`, `TypedTestSpec`, `TypedLogSpec`.
//!
//! Saída final do Pass 2 (inference) no que tange ao agrupamento por módulo:
//! o entry point tipado, as funções nomeadas e as actions. O codegen e o
//! tree shaking consomem estes tipos. A TAST em si (nó de expressão
//! `TypedExpr`/`TypedExprKind` e auxiliares de CSP) vive em
//! [`crate::typed`].

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::{Ty, TypeEnv};

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
    /// Especificação de logging `@log`. None se a função não tem `@log`.
    pub log: Option<TypedLogSpec>,
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
    /// um wrapper por spec (exceto negativos CompileError).
    pub tests: Vec<TypedTestSpec>,
    /// Especificação de logging `@log`. None se a action não tem `@log`.
    pub log: Option<TypedLogSpec>,
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
}

/// Especificação de logging `@log` tipada — pronta para o codegen.
///
/// O typeck processa o template `msg` e produz `msg_expr` (expressão tipada
/// que produz `Text` — cadeia de `text_replace_first` via `infer_format`).
/// O codegen injeta `kata_rt_log_publish` no prólogo (`Enter`) ou
/// epílogo (`Exit`) com o valor SSA de `msg_expr`.
#[derive(Debug, Clone)]
pub enum TypedLogSpec {
    /// Loga no prólogo (entrada). Placeholders só podem referenciar params.
    Enter {
        msg_expr: Spanned<TypedExpr>,
        topic: Option<String>,
        policy: Option<String>,
        level: i64,
    },
    /// Loga no epílogo (saída). Placeholders podem referenciar params e vars do corpo.
    Exit {
        msg_expr: Spanned<TypedExpr>,
        topic: Option<String>,
        policy: Option<String>,
        level: i64,
    },
}
