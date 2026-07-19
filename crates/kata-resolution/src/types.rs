//! Tipos de dados do pass de resolution.
//!
//! Structs e enums produzidos pelo resolution e consumidos pelo inference:
//! - `ResolvedModule`: TypeEnv populado + assinaturas coletadas
//! - `Signature`: assinatura de função coletada no Pass 1
//! - `FunctionDef`: função nomeada com corpo Kata
//! - `ActionDef`: Action com body Kata
//! - `RefinedDeclInfo`: tipo refinado declarado pelo usuário
//! - `EnumPredDeclInfo`: enum com variantes predicadas
//! - `EnumPredVariant`: variante de enum predicado
//! - `ResolveError`: erro de resolution

use kata_ast::{ActionStmt, Expr, LambdaClause, Spanned};
use kata_core::{EnumRegistry, InterfaceRegistry, StructRegistry, Ty, TypeEnv};

/// Resultado da resolution — TypeEnv populado + assinaturas coletadas.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub type_env: TypeEnv,
    pub signatures: Vec<Signature>,
    /// Catálogo de variantes por enum.
    pub enum_registry: EnumRegistry,
    /// Catálogo de structs com campos e offsets.
    pub struct_registry: StructRegistry,
    /// Declarações refined pendentes para o inference sintetizar
    /// funções predicado e smart constructors falíveis.
    pub refined_decls: Vec<RefinedDeclInfo>,
    /// Enums com variantes predicadas pendentes para o inference
    /// sintetizar o construtor despachador.
    pub enum_pred_decls: Vec<EnumPredDeclInfo>,
    /// Catálogo de interfaces e implementações.
    pub interface_registry: InterfaceRegistry,
    /// Funções nomeadas com corpo Kata.
    /// Cada entrada preserva as cláusulas lambda para o inference processar.
    pub functions: Vec<FunctionDef>,
    /// Actions definidas no módulo.
    pub actions: Vec<ActionDef>,
}

/// Assinatura de função coletada no Pass 1.
#[derive(Debug, Clone)]
pub struct Signature {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub ffi_symbol: Option<String>,
    pub is_associative: bool,
    pub associative_neutral: Option<i64>,
    /// Se `true`, esta assinatura é uma Action.
    /// Actions são chamadas com `!` e têm `is_action = true` no DispatchTable.
    pub is_action: bool,
    /// Se `true`, o dispatch tenta args invertidos quando 0 candidatos
    /// compatíveis são encontrados e arity == 2. Populado pela diretiva
    /// `@commutative` na resolution.
    pub is_commutative: bool,
    /// Type params da assinatura genérica (ex: `["T"]` para `id :: T => T`).
    /// Vazio para funções não-genéricas. Coletado examinando os `Ty::Var` em
    /// param_types e return_type cujo nome é UPPER_CASE e não está no TypeEnv.
    pub type_params: Vec<String>,
}

/// Definição de função nomeada com corpo Kata (não-FFI).
///
/// Produzida no resolution quando `Item::Sig` tem `body = Some(clauses)`.
/// O inference consome as cláusulas e produz `TypedExprKind::Lambda` com
/// `func_name = Some(name)` e os tipos da assinatura.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub clauses: Vec<Spanned<LambdaClause>>,
}

/// Definição de Action com body Kata.
///
/// Produzida no resolution quando `Item::ActionDecl` é encontrado.
/// O inference consome o body e produz `TypedAction`.
#[derive(Debug, Clone)]
pub struct ActionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub body: Vec<ActionStmt>,
}

/// Informação de um tipo refinado declarado pelo usuário.
/// O inference sintetiza funções predicado e smart constructor falível a partir desta info.
#[derive(Debug, Clone)]
pub struct RefinedDeclInfo {
    /// Nome do tipo refinado (ex: "PositiveInt").
    pub name: String,
    /// Tipo base (ex: Ty::Prim(PrimTy::Int)).
    pub base_ty: Ty,
    /// Predicados como `Spanned<Expr>` (com Hole como placeholder).
    pub predicates: Vec<Spanned<Expr>>,
}

/// Informação de um enum com variantes predicadas.
/// O inference sintetiza o construtor que despacha para a variante correta.
#[derive(Debug, Clone)]
pub struct EnumPredDeclInfo {
    /// Nome do enum (ex: "IMC").
    pub name: String,
    /// Tipo do payload comum a todas as variantes (ex: Ty::Prim(PrimTy::Float)).
    pub payload_ty: Ty,
    /// Variantes predicadas: (nome, predicado, tag).
    /// A última variante (sem predicado) é o fallback/default.
    pub variants: Vec<EnumPredVariant>,
}

/// Variante de um enum predicado.
#[derive(Debug, Clone)]
pub struct EnumPredVariant {
    /// Nome da variante (ex: "Magreza").
    pub name: String,
    /// Predicado como `Spanned<Expr>` (com Hole como placeholder).
    /// None = variante default/fallback.
    pub predicate: Option<Spanned<Expr>>,
    /// Tag da variante no enum (índice na declaração).
    pub tag: usize,
}

/// Erro de resolution (wrapped FrontendError/MiddleError).
#[derive(Debug, Clone)]
pub enum ResolveError {
    UnknownType {
        name: String,
    },
    UnknownFfi {
        name: String,
    },
    DuplicateSignature {
        name: String,
    },
    /// Diretiva não reconhecida no contexto (Sig, Action, ou Implements method).
    /// `name` é o nome da diretiva (ex: "tset"), `context` é onde apareceu
    /// (ex: "action", "sig", "implements method"), `item_name` é o nome do
    /// item onde a diretiva foi usada (ex: nome da action ou sig).
    UnknownDirective {
        name: String,
        context: &'static str,
        item_name: String,
    },
}
