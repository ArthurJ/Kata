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
    /// Nomes dos params. `Some(nome)` para params nomeados (`x::Tipo`),
    /// `None` para posicional. Vazio para funções puras/FFI sem nomes.
    /// Usado pelo typeck para mapear DictLit args → params nomeados.
    pub param_names: Vec<Option<String>>,
}

/// Especificação de logging `@log` anotada em Action ou função nomeada.
///
/// Produzida no resolution a partir das diretivas `@log`.
/// O inference consome e produz `TypedLogSpec` em `TypedAction`/`TypedFunction`.
/// O codegen injeta `kata_rt_log_publish` no prólogo (`when: "enter"`) ou
/// epílogo (`when: "exit"`).
#[derive(Debug, Clone)]
pub struct LogSpec {
    /// Template compile-time. `{expr}` interpola. `{{` escapa `{`.
    pub msg: String,
    /// `"enter"` = loga no prólogo. `"exit"` = loga no epílogo. Obrigatório.
    pub when: String,
    /// Tópico (nome do canal CSP). None = usar config herdada do fiber.
    /// Mutuamente exclusivo com `file`.
    pub topic: Option<String>,
    /// Nome do identificador File para write direto (ex: `stdout`).
    /// Mutuamente exclusivo com `topic`. O inference resolve como
    /// `Expr::Ident(name)` e tipa como `Ty::File`.
    pub file: Option<String>,
    /// Política: `"drop"` ou `"block"`. None = usar config herdada.
    /// Só válido com `topic` (não com `file`).
    pub policy: Option<String>,
    /// Level como variante do enum LogLevel (ex: `"Info"`). None = Info default.
    pub level: Option<String>,
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
/// O inference consome as cláusulas e produz `TypedExprKind::Lambda` com
/// `func_name = Some(name)` e os tipos da assinatura.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub clauses: Vec<Spanned<LambdaClause>>,
    /// Especificações de logging `@log`. Múltiplas diretivas `@log` são
    /// suportadas — cada uma injeta independentemente no prólogo/epílogo.
    /// Vazio se a função não tem `@log`.
    pub log: Vec<LogSpec>,
    /// Especificação de cache `@cache{strategy: "LRU"}`. None se a função
    /// não tem `@cache`.
    pub cache_strategy: Option<String>,
    /// Especificação de timer `@timer`. None se a função não tem `@timer`.
    pub timer: Option<TimerSpec>,
    /// Nomes das diretivas customizadas aplicadas a esta função (em ordem).
    /// Preenchido pelo resolution, consumido pelo `desugar_directives`.
    pub custom_directives: Vec<String>,
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
    pub return_type: Ty,
    pub body: Vec<ActionStmt>,
    /// Casos de teste anotados com `@test`. Cada `@test` vira um `TestSpec`
    /// cujos args são `Expr` não-tipado — o inference tipa via `infer_expr`.
    /// Vazio quando a action não tem `@test`.
    pub tests: Vec<TestSpec>,
    /// Especificações de logging `@log`. Múltiplas diretivas `@log` são
    /// suportadas — cada uma injeta independentemente no prólogo/epílogo.
    /// Vazio se a action não tem `@log`.
    pub log: Vec<LogSpec>,
    /// Nomes das diretivas customizadas aplicadas a esta action (em ordem).
    /// Preenchido pelo resolution, consumido pelo `desugar_directives`.
    pub custom_directives: Vec<String>,
}

/// Especificação de um caso de teste `@test` anotado em uma action.
///
/// Produzida no resolution a partir das diretivas `@test` da `ActionDecl`.
/// O inference tipa `args` (`Spanned<Expr>` → `Spanned<TypedExpr>`) e
/// propaga o `TestSpec` tipado em `TypedAction.tests`. O codegen gera
/// um wrapper `__kata_test_*` por `TestSpec` (exceto negativos CompileError).
///
/// - `desc`: identificação do teste no relatório do driver.
/// - `args`: argumentos literais para chamar a action (None = Unit).
/// - `timeout`: timeout em ms para o runner (`kata_rt_set_test_timeout`).
/// - `expects`: mensagem esperada de erro. Prefixo `"CompileError:"` marca
///   teste negativo — o codegen NÃO gera wrapper; o driver tenta compilar
///   o sub-módulo isolado e verifica a falha.
#[derive(Debug, Clone)]
pub struct TestSpec {
    pub desc: Option<String>,
    pub args: Option<Spanned<Expr>>,
    pub timeout: Option<i64>,
    pub expects: Option<String>,
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

    /// `directive` e `action` com o mesmo nome no mesmo escopo —
    /// namespaces disjuntos (PRD D12).
    #[error("diretiva `{name}` e action com mesmo nome no mesmo escopo — namespace disjunto")]
    #[diagnostic(code = "resolve.directive_action_name_conflict")]
    DirectiveActionNameConflict { name: String },
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

/// Chave do DirectiveRegistry: (nome, when, on).
/// Diretivas com mesmo nome coexistem se tiverem (when, on) diferentes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectiveKey {
    pub name: String,
    pub when: Hook,
    pub on: Target,
}

/// Definição de uma diretiva customizada — body que será inlined.
#[derive(Debug, Clone)]
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
    pub fn has_compatible_target(&self, name: &str, item_target: Target) -> bool {
        self.entries
            .iter()
            .filter(|(k, _)| k.name == name)
            .any(|(k, _)| k.on == item_target || k.on == Target::Any)
    }

    /// Mescla outro registry neste, preservando overloads por `(when, on)`.
    /// Diretivas com mesma chave `(nome, when, on)` → conflito (erro).
    /// Diretivas com mesmo nome mas `(when, on)` diferente coexistem.
    pub fn merge(&mut self, other: DirectiveRegistry) -> Vec<ResolveError> {
        let mut errors = Vec::new();
        for (key, def) in other.entries {
            match self.entries.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
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
    /// `Target::Function` para o mesmo `(nome, when)` (PRD regra 2.5.4).
    ///
    /// Deve ser chamado após todas as inserções (Pass 0.5 completo).
    /// Retorna erros para cada conflito encontrado.
    pub fn validate_any_conflicts(&self) -> Vec<ResolveError> {
        use std::collections::HashMap;
        // Agrupa por (nome, when) → set de Targets encontrados.
        let mut groups: HashMap<(&str, Hook), Vec<Target>> = HashMap::new();
        for k in self.entries.keys() {
            groups
                .entry((k.name.as_str(), k.when))
                .or_default()
                .push(k.on);
        }
        let mut errors = Vec::new();
        for ((name, when), targets) in &groups {
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
