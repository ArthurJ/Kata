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
use kata_core::{EnumRegistry, InterfaceRegistry, RefinesRegistry, StructRegistry, Ty, TypeEnv};
use std::collections::HashMap;
use thiserror::Error;

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
    /// Catálogo de delegações `refines` — tipo refined → interfaces delegadas.
    pub refines_registry: RefinesRegistry,
    /// Funções nomeadas com corpo Kata.
    /// Cada entrada preserva as cláusulas lambda para o inference processar.
    pub functions: Vec<FunctionDef>,
    /// Actions definidas no módulo.
    pub actions: Vec<ActionDef>,
    /// Registro de diretivas customizadas declaradas no módulo.
    pub directive_registry: DirectiveRegistry,
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

/// Especificação de timer `@timer` anotada em função nomeada.
///
/// Produzida no resolution a partir da diretiva `@timer`.
/// O inference consome e propaga em `TypedFunction.timer_spec`.
/// O codegen injeta `kata_rt_timer_now()` no prólogo (start) e
/// `kata_rt_timer_now() - start` no epílogo (delta), publicando via
/// `kata_rt_log_publish`.
#[derive(Debug, Clone)]
pub struct TimerSpec {
    /// Tópico (canal de output). None = nome da função.
    pub topic: Option<String>,
    /// Se `true`, agrega min/max/mean sobre `repeat` amostras.
    pub stats: bool,
    /// Janela de amostras antes de publicar.
    pub repeat: u32,
    /// Template da mensagem. None = default conforme `stats`.
    pub msg: Option<String>,
}

/// Definição de função nomeada com corpo Kata (não-FFI).
///
/// Produzida no resolution quando `Item::Sig` tem `body = Some(clauses)`.
/// Aplicação de uma diretiva customizada no site de uso (`@nome{...}`).
/// Carrega o nome da diretiva e os args fornecidos no site de aplicação.
/// O desugar injeta `let _<key> := <value>` para cada arg antes do body da diretiva.
#[derive(Debug, Clone)]
pub struct CustomDirectiveApp {
    /// Nome da diretiva aplicada (ex: "trace").
    pub name: String,
    /// Args nomeados do site de aplicação, em ordem (ex: msg, when, topic).
    /// `when` é consumido como seletor de overload, os demais viram bindings.
    pub args: Vec<kata_ast::DirectiveArg>,
    /// Chaves dos args nomeados, excluindo `when` e `on` (metadados de despacho).
    /// Usado para despachar a declaration correta por combinação de args.
    pub arg_keys: Vec<String>,
    /// `when` do site de aplicação como `Hook`, se presente.
    /// Usado para despachar a declaration correta por hook (Enter vs Exit).
    pub site_when: Option<Hook>,
}

/// O inference consome as cláusulas e produz `TypedExprKind::Lambda` com
/// `func_name = Some(name)` e os tipos da assinatura.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub clauses: Vec<Spanned<LambdaClause>>,
    /// Especificação de cache `@cache{strategy: "LRU"}`. None se a função
    /// não tem `@cache`.
    pub cache_strategy: Option<String>,
    /// Especificação de timer `@timer`. None se a função não tem `@timer`.
    pub timer: Option<TimerSpec>,
    /// Diretivas customizadas aplicadas a esta função (em ordem).
    /// Preenchido pelo resolution, consumido pelo `desugar_directives`.
    pub custom_directives: Vec<CustomDirectiveApp>,
}

/// Definição de Action com body Kata.
///
/// Produzida no resolution quando `Item::ActionDecl` é encontrado.
/// O inference consome o body e produz `TypedAction`.
#[derive(Debug, Clone)]
pub struct ActionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    /// Nomes dos params. `Some(nome)` para params nomeados (`x::Tipo`),
    /// `None` para posicional legado (não usado após migração total).
    pub param_names: Vec<Option<String>>,
    /// Defaults dos params. `None` = obrigatório, `Some(expr)` = tem default.
    /// Paralelo a `param_names`. Vazio para actions sem defaults.
    pub param_defaults: Vec<Option<Spanned<Expr>>>,
    pub return_type: Ty,
    pub body: Vec<ActionStmt>,
    /// Casos de teste anotados com `@test`. Cada `@test` vira um `TestSpec`
    /// cujos args são `Expr` não-tipado — o inference tipa via `infer_expr`.
    /// Vazio quando a action não tem `@test`.
    pub tests: Vec<TestSpec>,
    /// Diretivas customizadas aplicadas a esta action (em ordem).
    /// Preenchido pelo resolution, consumido pelo `desugar_directives`.
    pub custom_directives: Vec<CustomDirectiveApp>,
}

/// Especificação de um caso de teste `@test` anotado em uma action.
///
/// Produzida no resolution a partir das diretivas `@test` da `ActionDecl`.
/// O inference tipa `args` (`Spanned<Expr>` → `Spanned<TypedExpr>`) e
/// propaga o `TestSpec` tipado em `TypedAction.tests`. O codegen gera
/// um wrapper `__kata_test_*` por `TestSpec`.
///
/// - `desc`: identificação do teste no relatório do driver.
/// - `args`: argumentos literais para chamar a action (None = Unit).
/// - `timeout`: timeout em ms para o runner (`kata_rt_set_test_timeout`).
/// - `expects`: string esperada do `show` do payload de `Result::Err`.
///   Verificada pelo wrapper via `policy`. Se `None`, não verifica.
/// - `policy`: política de match entre `show(err)` e `expects`.
///   Default `Exact` quando `expects` está presente e `policy` é omitido.
#[derive(Debug, Clone)]
pub struct TestSpec {
    pub desc: Option<String>,
    pub args: Option<Spanned<Expr>>,
    pub timeout: Option<i64>,
    pub expects: Option<String>,
    pub policy: Option<MatchPolicy>,
}

/// Política de match entre `show(err)` e a string `expects`.
///
/// - `Exact`: `show(err) == expects`
/// - `Prefix`: `show(err).starts_with(expects)`
/// - `Contains`: `show(err).contains(expects)`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPolicy {
    Exact,
    Prefix,
    Contains,
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
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum ResolveError {
    #[error("tipo desconhecido: `{name}`")]
    #[diagnostic(code = "resolve.unknown_type")]
    UnknownType { name: String },

    #[error("símbolo FFI desconhecido: `{name}`")]
    #[diagnostic(code = "resolve.unknown_ffi")]
    UnknownFfi { name: String },

    #[error("assinatura duplicada: `{name}`")]
    #[diagnostic(code = "resolve.duplicate_signature")]
    DuplicateSignature { name: String },

    /// Diretiva não reconhecida no contexto (Sig, Action, ou Implements method).
    /// `name` é o nome da diretiva (ex: "tset"), `context` é onde apareceu
    /// (ex: "action", "sig", "implements method"), `item_name` é o nome do
    /// item onde a diretiva foi usada (ex: nome da action ou sig).
    #[error("diretiva `{name}` não reconhecida em {context} `{item_name}`")]
    #[diagnostic(code = "resolve.unknown_directive")]
    UnknownDirective {
        name: String,
        context: &'static str,
        item_name: String,
    },

    /// `refines` aplicado a tipo não-refined, ou base não implementa a interface.
    #[error("refines inválido para `{type_name}`: {reason}")]
    #[diagnostic(code = "resolve.invalid_refines")]
    InvalidRefines { type_name: String, reason: String },

    /// Diretiva customizada com (nome, when, on) duplicado.
    #[error("diretiva `{name}` duplicada: when={when}, on={on}")]
    #[diagnostic(code = "resolve.duplicate_directive")]
    DuplicateDirective {
        name: String,
        when: String,
        on: String,
    },

    /// Diretiva customizada declarada mas não aplicável ao tipo do item.
    #[error("diretiva `{name}` não pode decorar {item_kind} (on={on})")]
    #[diagnostic(code = "resolve.directive_target_mismatch")]
    DirectiveTargetMismatch {
        name: String,
        item_kind: String,
        on: String,
    },

    /// `Target::Any` coexiste com `Target::Action` ou `Target::Function`
    /// para o mesmo `(nome, when)` — a regra do PRD proíbe a mistura.
    #[error("diretiva `{name}` mistura Target::Any com específico para when={when}")]
    #[diagnostic(code = "resolve.directive_any_conflict")]
    DirectiveAnyConflict { name: String, when: String },
}

/// Formata um `Vec<ResolveError>` como string legível (erros separados por `; `).
/// `Vec` não implementa `Display`, então sem este helper o caller teria que
/// usar `{e:?}` (Debug), que mostraria spans brutos.
pub(crate) fn format_resolve_errors(errors: &[ResolveError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

// ── Diretivas customizadas ──────────────────────────────────────────

/// Hook — ponto do ciclo de vida onde a diretiva injeta.
/// Resolvido de `Expr::VariantQual { enum_name: "Hook", variant: "..." }`
/// contra `enum Hook` no prelude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hook {
    Enter,
    Exit,
    ShortCircuit,
    Transform,
}

/// Target — tipo de item que a diretiva pode decorar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Action,
    Function,
    Any,
}

/// Chave do DirectiveRegistry: (nome, when, on, arg_keys).
/// Diretivas com mesmo nome coexistem se tiverem (when, on, arg_keys) diferentes.
/// `arg_keys` é a lista ordenada de chaves de args que a declaration aceita
/// no site de aplicação (ex: `["msg"]` vs `["msg", "topic"]`), excluindo
/// `when` e `on` que são metadados de despacho.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectiveKey {
    pub name: String,
    pub when: Hook,
    pub on: Target,
    pub arg_keys: Vec<String>,
}

/// Definição de uma diretiva customizada — body que será inlined.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectiveDef {
    pub key: DirectiveKey,
    /// Body da diretiva — statements copiados para o item decorado.
    pub body: Vec<ActionStmt>,
}

/// Registro de diretivas customizadas.
/// Chave: (nome, when, on). Diretivas com mesmo nome coexistem
/// se tiverem (when, on) diferentes — overloading por Hook e Target.
#[derive(Debug, Clone, Default)]
pub struct DirectiveRegistry {
    pub entries: HashMap<DirectiveKey, DirectiveDef>,
}

impl DirectiveRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insere uma diretiva. Retorna erro se (nome, when, on) já existe.
    pub fn insert(&mut self, def: DirectiveDef) -> Result<(), ResolveError> {
        if self.entries.contains_key(&def.key) {
            return Err(ResolveError::DuplicateDirective {
                name: def.key.name,
                when: format!("{:?}", def.key.when),
                on: format!("{:?}", def.key.on),
            });
        }
        self.entries.insert(def.key.clone(), def);
        Ok(())
    }

    /// Busca todas as diretivas com o nome dado (para qualquer when/on).
    pub fn lookup_by_name(&self, name: &str) -> Vec<&DirectiveDef> {
        self.entries
            .iter()
            .filter(|(k, _)| k.name == name)
            .map(|(_, v)| v)
            .collect()
    }

    /// Verifica se existe alguma diretiva com o nome dado.
    pub fn contains_name(&self, name: &str) -> bool {
        self.entries.keys().any(|k| k.name == name)
    }

    /// Verifica se o nome tem pelo menos uma def com Target compatível
    /// com o kind do item decorado (Function ou Action).
    /// `item_target` é `Target::Function` para Sig, `Target::Action` para ActionDecl.
    /// Não considera `arg_keys` — só valida Target por nome.
    pub fn has_compatible_target(&self, name: &str, item_target: Target) -> bool {
        self.entries
            .iter()
            .filter(|(k, _)| k.name == name)
            .any(|(k, _)| k.on == item_target || k.on == Target::Any)
    }

    /// Busca a declaration que casa com o site de aplicação: mesmo nome,
    /// Target compatível, e exatamente os mesmos `arg_keys`.
    /// Retorna `None` se nenhuma declaration casar.
    pub fn lookup_by_application(
        &self,
        name: &str,
        item_target: Target,
        arg_keys: &[String],
    ) -> Option<&DirectiveDef> {
        self.entries
            .iter()
            .filter(|(k, _)| k.name == name && (k.on == item_target || k.on == Target::Any))
            .find(|(k, _)| k.arg_keys.as_slice() == arg_keys)
            .map(|(_, v)| v)
    }

    /// Mescla outro registry neste, preservando overloads por `(when, on, arg_keys)`.
    /// Diretivas com mesma chave `(nome, when, on, arg_keys)` → conflito (erro),
    /// exceto quando o body é idêntico (mesma declaration vinda do prelude via
    /// merge em módulo importado) — neste caso é no-op silencioso.
    /// Diretivas com mesmo nome mas `(when, on, arg_keys)` diferente coexistem.
    pub fn merge(&mut self, other: DirectiveRegistry) -> Vec<ResolveError> {
        let mut errors = Vec::new();
        for (key, def) in other.entries {
            match self.entries.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    // Se o body é idêntico, é a mesma declaration do prelude
                    // chegando via merge de módulo importado — no-op.
                    if entry.get().body == def.body {
                        continue;
                    }
                    errors.push(ResolveError::DuplicateDirective {
                        name: entry.key().name.clone(),
                        when: format!("{:?}", entry.key().when),
                        on: format!("{:?}", entry.key().on),
                    });
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(def);
                }
            }
        }
        errors
    }

    /// Valida que `Target::Any` não coexiste com `Target::Action` ou
    /// `Target::Function` para o mesmo `(nome, when, arg_keys)`.
    ///
    /// Deve ser chamado após todas as inserções (Pass 0.5 completo).
    /// Retorna erros para cada conflito encontrado.
    pub fn validate_any_conflicts(&self) -> Vec<ResolveError> {
        use std::collections::HashMap;
        // Agrupa por (nome, when, arg_keys) → set de Targets encontrados.
        let mut groups: HashMap<(&str, Hook, &Vec<String>), Vec<Target>> = HashMap::new();
        for k in self.entries.keys() {
            groups
                .entry((k.name.as_str(), k.when, &k.arg_keys))
                .or_default()
                .push(k.on);
        }
        let mut errors = Vec::new();
        for ((name, when, _arg_keys), targets) in &groups {
            let has_any = targets.iter().any(|t| matches!(t, Target::Any));
            let has_specific = targets
                .iter()
                .any(|t| matches!(t, Target::Action | Target::Function));
            if has_any && has_specific {
                errors.push(ResolveError::DirectiveAnyConflict {
                    name: name.to_string(),
                    when: format!("{when:?}"),
                });
            }
        }
        errors
    }
}
