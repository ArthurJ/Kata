//! `Ty` — tipo canônico do compilador.
//!
//! O typeck produz `Ty` em cada `TypedExpr.ty`. O lowering mapeia `Ty`
//! direto para ABI do Cranelift (Int→I64, Float→F64, Text/Struct/Sum→Ptr).
//!
//! `PrimTy` é o mapeamento de representação FFI (não tipo da linguagem).
//! Os tipos da linguagem (`Int`, `Float`, `Text`, `Rational`) são `data`
//! com `@ffi` no prelude. `Boolean` é `enum` no prelude.

use std::collections::{HashMap, HashSet};

/// Tipo canônico do compilador.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Int, Float, Text, Rational — tipos opacos do prelude com @ffi.
    Prim(PrimTy),
    /// `()` — unit.
    Unit,
    /// Tipo struct (produto) — `data Pessoa (nome::Text idade::Int)`.
    Struct(String),
    /// Tipo soma (enum) — `enum Boolean { True, False }`.
    Sum(String),
    /// Tipo função — `(A B -> C)`.
    Function(Vec<Ty>, Box<Ty>),
    /// Action como valor first-class.
    /// params: tipos dos parâmetros (sem nomes).
    /// ret: tipo de retorno.
    /// Separada de Ty::Function porque as ABIs são semanticamente diferentes:
    /// Function: (captures_ptr, args) -> ret — pura, sem scheduler
    /// Action: (fiber_arena, caller_arena, args_ptr) -> i64 — impura, scheduler M:N
    Action(Vec<Ty>, Box<Ty>),
    /// Antecipado. Sem field access, sem .N — só tipo estrutural
    /// para suportar tuple patterns em match/lambda.
    Tuple(Vec<Ty>),
    /// Variável de inferência — preenchida pelo typeck.
    InferVar(u32),
    /// Variável de tipo nomeada pelo usuário (ex: `T`, `E` em `Result::(T, E)`).
    /// Distinta de InferVar — Var é parâmetro de tipo nomeado, InferVar é
    /// gerada internamente pelo typeck.
    Var(String),
    /// Tipo genérico instanciado (ex: `Result<Int, Text>`).
    /// Carrega o nome do enum + os argumentos de tipo concretos.
    Generic(String, Vec<Ty>),
    /// Interface (ex: `NUM`, `ORD`, `EQ`, `SHOW`).
    /// Usada como tipo de parâmetro em assinaturas para indicar
    /// "qualquer tipo que implementa esta interface". Não é um tipo
    /// concreto — o dispatch resolve para o tipo real no call site.
    Interface(String),
    /// Lista persistente: `[T]` — Cons cell encadeada.
    List(Box<Ty>),
    /// Array contíguo: `{T}` — bloco imutável.
    Array(Box<Ty>),
    /// Range lazy: `[a..s..b]` — start, step, end. Genérico sobre A.
    Range(Box<Ty>),
    /// Dict persistente: `Dict::(K, V)` — HAMT de pares chave-valor.
    Dict(Box<Ty>, Box<Ty>),
    /// Set persistente: `Set::T` — HAMT de chaves (sem values).
    Set(Box<Ty>),
    /// Sender de canal — `Sender::T`. Pode fazer `!>`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Sender(Box<Ty>),
    /// Receiver de canal — `Receiver::T`. Pode fazer `<!`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Receiver(Box<Ty>),
    /// Fábrica de receivers para broadcast — `ReceiverFactory::T`.
    /// Chamada como action produz `Receiver::T`.
    ReceiverFactory(Box<Ty>),
    /// Byte — unidade de 8 bits (0x00-0xFF). Distinto de Int.
    /// SMI-tagged no runtime (como Int). Conversão explícita via `int()` / `byte()`.
    Byte,
    /// Bytes — sequência contígua de u8. Blob opaco para I/O e marshalling.
    /// Ponteiro na ABI (como Array, Text, Struct). Imutável.
    Bytes,
    /// File — handle opaco para arquivo aberto. Sem parametrização de tipo
    /// (encoding é determinado pela operação: read → Bytes, readline → Text).
    /// Ponteiro na ABI (i64 com tag). Generaliza para Socket no futuro via
    /// camada IoHandle comum no runtime.
    File,
    /// Socket — handle opaco para socket TCP/Unix aberto. Sem parametrização
    /// de tipo (encoding determinado pela operação: read → Bytes, write aceita
    /// Text ou Bytes). Ponteiro na ABI (i64). Pode ser Listener (passivo) ou
    /// Connected (ativo, full-duplex).
    Socket,
}

/// Mapeamento de representação FFI.
/// Não é tipo da linguagem — é como o codegen mapeia `Ty → ABI`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    /// Int → i64 (BigInt/SMI no runtime).
    Int,
    /// Float → f64.
    Float,
    /// Text → ponteiro para kata_rt_string.
    Text,
    /// Rational → ponteiro para kata_rt_rat.
    Rational,
}

impl PrimTy {
    /// Nome do símbolo FFI de representação.
    pub fn ffi_repr(self) -> &'static str {
        match self {
            PrimTy::Int => "i64",
            PrimTy::Float => "f64",
            PrimTy::Text => "kata_rt_string",
            PrimTy::Rational => "kata_rt_rat",
        }
    }

    /// Constrói PrimTy a partir do nome de símbolo FFI.
    pub fn from_ffi(s: &str) -> Option<PrimTy> {
        match s {
            "i64" => Some(PrimTy::Int),
            "f64" => Some(PrimTy::Float),
            "kata_rt_string" => Some(PrimTy::Text),
            "kata_rt_rat" => Some(PrimTy::Rational),
            _ => None,
        }
    }
}

impl Ty {
    /// `Ty::Prim(PrimTy::Int)` — conveniência.
    pub fn int() -> Ty {
        Ty::Prim(PrimTy::Int)
    }

    /// `Ty::Prim(PrimTy::Float)` — conveniência.
    pub fn float() -> Ty {
        Ty::Prim(PrimTy::Float)
    }

    /// `Ty::Prim(PrimTy::Text)` — conveniência.
    pub fn text() -> Ty {
        Ty::Prim(PrimTy::Text)
    }

    /// `Ty::Prim(PrimTy::Rational)` — conveniência.
    pub fn rational() -> Ty {
        Ty::Prim(PrimTy::Rational)
    }

    /// `Ty::Sum("Boolean")` — conveniência para Boolean.
    pub fn boolean() -> Ty {
        Ty::Sum("Boolean".into())
    }
}

/// Binding de tipo com origem (módulo onde foi definido).
///
/// `origin` identifica de qual módulo o binding veio ("core", "mock_math",
/// "my_module" para definido localmente). Todo `TypeBinding` tem um "dono".
/// Tipos locais usam o nome do próprio módulo como `origin`.
#[derive(Debug, Clone)]
pub struct TypeBinding {
    pub ty: Ty,
    pub origin: String,
    /// Se o binding é `let g := soma` (Ident apontando para função nomeada no
    /// DispatchTable), guarda o nome da função original (`"soma"`). Usado pela
    /// reflexão para distinguir alias de função nomeada (caso dinâmico escalar)
    /// de lambda com binding (caso estático lista).
    pub fn_alias: Option<String>,
}

/// Árvore de escopos para name resolution.
///
/// Populada no resolution (Pass 0+1) e consumida no inference (Pass 2).
/// Não sobrevive além do typeck — a TAST já carrega os tipos resolvidos.
#[derive(Debug, Clone)]
pub struct TypeEnv {
    bindings: HashMap<String, TypeBinding>,
    parent: Option<Box<TypeEnv>>,
    /// Nomes declarados como mutáveis (`var`) neste escopo.
    /// Necessário para validar reatribuição (`x := 42` só é válido se `x`
    /// foi declarado com `var`, não `let`).
    mutables: HashSet<String>,
    /// Nomes com conflito de origin entre imports (ambiguidade).
    /// `resolve_type_expr` deve erroar ao usar estes sem qualificar.
    ambiguous: HashSet<String>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: None,
            mutables: HashSet::new(),
            ambiguous: HashSet::new(),
        }
    }

    pub fn with_parent(parent: TypeEnv) -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: Some(Box::new(parent)),
            mutables: HashSet::new(),
            ambiguous: HashSet::new(),
        }
    }

    /// Define um nome no escopo atual (binding imutável por default).
    pub fn define(&mut self, name: &str, ty: Ty, origin: &str) {
        self.bindings.insert(
            name.to_string(),
            TypeBinding {
                ty,
                origin: origin.to_string(),
                fn_alias: None,
            },
        );
    }

    /// Define um nome com `fn_alias` — usado por `let g := soma` para
    /// rastrear que `g` é um alias para a função nomeada `soma`.
    pub fn define_with_alias(&mut self, name: &str, ty: Ty, origin: &str, alias: Option<String>) {
        self.bindings.insert(
            name.to_string(),
            TypeBinding {
                ty,
                origin: origin.to_string(),
                fn_alias: alias,
            },
        );
    }

    /// Define um nome mutável no escopo atual (`var`).
    /// Marca o nome como mutável para validação de reatribuição.
    pub fn define_mutable(&mut self, name: &str, ty: Ty, origin: &str) {
        self.bindings.insert(
            name.to_string(),
            TypeBinding {
                ty,
                origin: origin.to_string(),
                fn_alias: None,
            },
        );
        self.mutables.insert(name.to_string());
    }

    /// Retorna o `fn_alias` de um binding, se houver. Percorre a cadeia
    /// de escopos.
    pub fn fn_alias_of(&self, name: &str) -> Option<&str> {
        if let Some(binding) = self.bindings.get(name)
            && let Some(ref alias) = binding.fn_alias
        {
            return Some(alias);
        }
        if let Some(ref parent) = self.parent {
            return parent.fn_alias_of(name);
        }
        None
    }

    /// Verifica se um nome foi declarado como mutável (`var`).
    /// Percorre a cadeia de escopos.
    pub fn is_mutable(&self, name: &str) -> bool {
        if self.mutables.contains(name) {
            return true;
        }
        self.parent.as_deref().is_some_and(|p| p.is_mutable(name))
    }

    /// Procura um nome na cadeia de escopos.
    /// Retorna apenas o `Ty` — sem `origin`.
    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        if let Some(binding) = self.bindings.get(name) {
            return Some(&binding.ty);
        }
        self.parent.as_deref().and_then(|p| p.lookup(name))
    }

    /// Procura um nome na cadeia de escopos, retornando o `TypeBinding`
    /// completo (com `origin`). Usado por `resolve_type_expr` para
    /// desambiguar tipos de módulos diferentes.
    pub fn lookup_binding(&self, name: &str) -> Option<&TypeBinding> {
        if let Some(binding) = self.bindings.get(name) {
            return Some(binding);
        }
        self.parent.as_deref().and_then(|p| p.lookup_binding(name))
    }

    /// Procura um nome na cadeia de escopos, filtrando por origin.
    ///
    /// Quando `module_path = Some(["core"])` qualifica um tipo (ex:
    /// `core.Result::Err`), este lookup retorna apenas o binding cuja
    /// `origin` corresponde. Percorre a cadeia de escopos do mais interno
    /// para o mais externo, retornando o primeiro binding que casa a
    /// origin. Se nenhum binding casa a origin, retorna `None`.
    pub fn lookup_with_origin(&self, name: &str, origin: &str) -> Option<&Ty> {
        if let Some(binding) = self.bindings.get(name)
            && binding.origin == origin
        {
            return Some(&binding.ty);
        }
        self.parent
            .as_deref()
            .and_then(|p| p.lookup_with_origin(name, origin))
    }

    /// Verifica se um nome está marcado como ambíguo (conflito de origin
    /// entre imports). Percorre a cadeia de escopos.
    pub fn is_ambiguous(&self, name: &str) -> bool {
        if self.ambiguous.contains(name) {
            return true;
        }
        self.parent.as_deref().is_some_and(|p| p.is_ambiguous(name))
    }

    /// Marca um nome como ambíguo neste escopo.
    pub fn mark_ambiguous(&mut self, name: &str) {
        self.ambiguous.insert(name.to_string());
    }

    /// Cria um escopo filho.
    pub fn push_scope(&self) -> TypeEnv {
        TypeEnv::with_parent(self.clone())
    }

    /// Drena os bindings (e mutables) de `other` para este escopo.
    ///
    /// Usado em `merge_resolved` para combinar o `type_env` do user module
    /// com o escopo filho do prelude. `other` fica vazio após a chamada.
    /// Conflitos de origin (mesmo nome, origins diferentes) são marcados
    /// como ambíguos.
    pub fn merge_bindings_from(&mut self, other: &mut TypeEnv) {
        for (name, binding) in other.bindings.drain() {
            if let Some(existing) = self.bindings.get(&name)
                && existing.origin != binding.origin
            {
                self.ambiguous.insert(name.clone());
            }
            self.bindings.insert(name, binding);
        }
        for name in other.mutables.drain() {
            self.mutables.insert(name);
        }
        for name in other.ambiguous.drain() {
            self.ambiguous.insert(name);
        }
    }

    /// Itera sobre os bindings deste escopo (não do parent).
    /// Retorna `(name, ty)` pares — usado pelo REPL `:env`.
    #[allow(dead_code)]
    pub(crate) fn local_bindings(&self) -> impl Iterator<Item = (&str, &Ty)> {
        self.bindings.iter().map(|(k, v)| (k.as_str(), &v.ty))
    }

    /// Itera sobre os bindings deste escopo (não do parent),
    /// retornando o `TypeBinding` completo (com `origin`).
    /// Usado por `merge_imports` para copiar tipos de módulos importados.
    pub fn local_bindings_full(&self) -> impl Iterator<Item = (&str, &TypeBinding)> {
        self.bindings.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Filtra bindings deste escopo (não do parent), mantendo apenas
    /// aqueles cujo nome está no `closure` ou cuja origin é `core` (prelude).
    /// Usado por `filter_exports` para esconder tipos não exportados.
    pub fn retain_by_closure(&mut self, closure: &std::collections::HashSet<String>) {
        self.bindings
            .retain(|name, binding| closure.contains(name) || binding.origin == "core");
    }

    /// Aplica substituições de type vars (ex: `T0 → List::Int`) a todos
    /// os bindings deste escopo E da cadeia de parents. Usado pelo `fork!`
    /// para propagar a unificação do tipo do canal quando o arg do fork
    /// contém `Ty::Var` e o param da action é concreto.
    pub fn apply_substitutions(&mut self, subs: &HashMap<String, Ty>) {
        for binding in self.bindings.values_mut() {
            binding.ty = apply_subs_to_ty(&binding.ty, subs);
        }
        if let Some(parent) = self.parent.as_mut() {
            parent.apply_substitutions(subs);
        }
    }
}

/// Aplica substituições de `Ty::Var(name)` → tipo concreto recursivamente.
/// Usado por `TypeEnv::apply_substitutions` para propagar unificação
/// de type vars do canal (ex: `T0 → List::Int`) nos bindings.
pub fn apply_subs_to_ty(ty: &Ty, subs: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Var(name) => subs.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Ty::List(elem) => Ty::List(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Array(elem) => Ty::Array(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Range(elem) => Ty::Range(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Sender(elem) => Ty::Sender(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Receiver(elem) => Ty::Receiver(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::ReceiverFactory(elem) => Ty::ReceiverFactory(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(apply_subs_to_ty(k, subs)),
            Box::new(apply_subs_to_ty(v, subs)),
        ),
        Ty::Set(elem) => Ty::Set(Box::new(apply_subs_to_ty(elem, subs))),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|e| apply_subs_to_ty(e, subs)).collect()),
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter().map(|a| apply_subs_to_ty(a, subs)).collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(|p| apply_subs_to_ty(p, subs)).collect(),
            Box::new(apply_subs_to_ty(ret, subs)),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params.iter().map(|p| apply_subs_to_ty(p, subs)).collect(),
            Box::new(apply_subs_to_ty(ret, subs)),
        ),
        _ => ty.clone(),
    }
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Display — sintaxe que o usuário escreve, não Debug de Rust.
// ---------------------------------------------------------------------------

impl std::fmt::Display for PrimTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimTy::Int => f.write_str("Int"),
            PrimTy::Float => f.write_str("Float"),
            PrimTy::Text => f.write_str("Text"),
            PrimTy::Rational => f.write_str("Rational"),
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Prim(p) => write!(f, "{p}"),
            Ty::Unit => f.write_str("()"),
            Ty::Var(name) => f.write_str(name),
            Ty::Sum(name) | Ty::Struct(name) | Ty::Interface(name) => f.write_str(name),
            Ty::Generic(name, args) => {
                f.write_str(name)?;
                f.write_str("::(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                f.write_str(")")
            }
            Ty::Tuple(elements) => {
                f.write_str("(")?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{elem}")?;
                }
                f.write_str(")")
            }
            Ty::Function(params, ret) => {
                f.write_str("(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{p}")?;
                }
                f.write_str(") -> ")?;
                write!(f, "{ret}")
            }
            Ty::Action(params, ret) => {
                f.write_str("Action(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{p}")?;
                }
                f.write_str(") -> ")?;
                write!(f, "{ret}")
            }
            Ty::List(inner) => write!(f, "[{inner}]"),
            Ty::Array(inner) => write!(f, "{{{inner}}}"),
            Ty::Range(inner) => write!(f, "[..{inner}]"),
            Ty::Dict(k, v) => write!(f, "Dict::({k}, {v})"),
            Ty::Set(t) => write!(f, "Set::{t}"),
            Ty::Sender(inner) => write!(f, "Sender::{inner}"),
            Ty::Receiver(inner) => write!(f, "Receiver::{inner}"),
            Ty::ReceiverFactory(inner) => write!(f, "ReceiverFactory::{inner}"),
            Ty::InferVar(_) => f.write_str("?"),
            Ty::Byte => f.write_str("Byte"),
            Ty::Bytes => f.write_str("Bytes"),
            Ty::File => f.write_str("File"),
            Ty::Socket => f.write_str("Socket"),
        }
    }
}

/// Formata uma lista de tipos como `(T1, T2, T3)` — útil para mensagens
/// de erro que mostram `Vec<Ty>` (ex: params de uma função, args de uma
/// chamada). Usa `Display` em cada elemento.
pub fn ty_list_to_string(tys: &[Ty]) -> String {
    tys.iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Ty {
    /// Formata o tipo na **sintaxe da linguagem** — o mesmo formato que o
    /// usuário escreve em assinaturas. Usado por `type!()` para produzir
    /// o `Text` que o usuário vê.
    ///
    /// Diferente de `Display` (que é para mensagens de erro e usa vírgulas
    /// em params de função), `display()` usa a sintaxe real da linguagem:
    /// `(Int Int -> Int)` em vez de `(Int, Int) -> Int`.
    pub fn display(&self) -> String {
        match self {
            Ty::Prim(p) => p.to_string(),
            Ty::Unit => "Unit".into(),
            Ty::Struct(name) | Ty::Sum(name) | Ty::Interface(name) => name.clone(),
            Ty::Var(name) => name.clone(),
            Ty::Generic(name, params) => {
                if params.len() == 1 {
                    format!("{name}::{}", params[0].display())
                } else {
                    let params_str = params
                        .iter()
                        .map(|p| p.display())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}::({params_str})")
                }
            }
            Ty::Function(params, ret) => {
                if params.is_empty() {
                    format!("Lambda(-> {})", ret.display())
                } else {
                    let params_str = params
                        .iter()
                        .map(|p| p.display())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("Lambda({params_str} -> {})", ret.display())
                }
            }
            Ty::Action(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Action({params_str}) => {}", ret.display())
            }
            Ty::Tuple(elements) => {
                let elems_str = elements
                    .iter()
                    .map(|e| e.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({elems_str})")
            }
            Ty::List(t) => format!("[{}]", t.display()),
            Ty::Array(t) => format!("{{{}}}", t.display()),
            Ty::Range(_) => "[a..s..b]".into(),
            Ty::Dict(k, v) => format!("Dict::({}, {})", k.display(), v.display()),
            Ty::Set(t) => format!("Set::{}", t.display()),
            Ty::Sender(t) => format!("Sender::{}", t.display()),
            Ty::Receiver(t) => format!("Receiver::{}", t.display()),
            Ty::ReceiverFactory(t) => format!("ReceiverFactory::{}", t.display()),
            Ty::InferVar(_) => panic!("type!() em tipo não-resolvido — bug do typeck"),
            Ty::Byte => "Byte".into(),
            Ty::Bytes => "Bytes".into(),
            Ty::File => "File".into(),
            Ty::Socket => "Socket".into(),
        }
    }

    /// Serializa `Ty` para texto — alias de `display()`.
    ///
    /// Usado pelo PRD de reflexão (`f.param_types`, `f.return_type`) para
    /// produzir a representação textual dos tipos na sidecar table e nos
    /// casos estáticos do typeck.
    pub fn to_text(&self) -> String {
        self.display()
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn display_prim_int() {
        assert_eq!(Ty::int().display(), "Int");
    }

    #[test]
    fn display_prim_float() {
        assert_eq!(Ty::float().display(), "Float");
    }

    #[test]
    fn display_prim_text() {
        assert_eq!(Ty::text().display(), "Text");
    }

    #[test]
    fn display_prim_rational() {
        assert_eq!(Ty::rational().display(), "Rational");
    }

    #[test]
    fn display_unit() {
        assert_eq!(Ty::Unit.display(), "Unit");
    }

    #[test]
    fn display_struct() {
        assert_eq!(Ty::Struct("Pessoa".into()).display(), "Pessoa");
    }

    #[test]
    fn display_sum() {
        assert_eq!(Ty::Sum("Boolean".into()).display(), "Boolean");
    }

    #[test]
    fn display_interface() {
        assert_eq!(Ty::Interface("NUM".into()).display(), "NUM");
    }

    #[test]
    fn display_var() {
        assert_eq!(Ty::Var("T".into()).display(), "T");
    }

    #[test]
    fn display_generic_single_param() {
        assert_eq!(
            Ty::Generic("Optional".into(), vec![Ty::int()]).display(),
            "Optional::Int"
        );
    }

    #[test]
    fn display_generic_multi_param() {
        assert_eq!(
            Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()]).display(),
            "Result::(Int, Text)"
        );
    }

    #[test]
    fn display_function() {
        assert_eq!(
            Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())).display(),
            "Lambda(Int Int -> Int)"
        );
    }

    #[test]
    fn display_function_no_params() {
        assert_eq!(
            Ty::Function(vec![], Box::new(Ty::int())).display(),
            "Lambda(-> Int)"
        );
    }

    #[test]
    fn display_action() {
        assert_eq!(
            Ty::Action(vec![Ty::int()], Box::new(Ty::Unit)).display(),
            "Action(Int) => Unit"
        );
    }

    #[test]
    fn display_action_multi_param() {
        assert_eq!(
            Ty::Action(vec![Ty::int(), Ty::text()], Box::new(Ty::int())).display(),
            "Action(Int, Text) => Int"
        );
    }

    #[test]
    fn display_tuple() {
        assert_eq!(
            Ty::Tuple(vec![Ty::int(), Ty::text()]).display(),
            "(Int, Text)"
        );
    }

    #[test]
    fn display_list() {
        assert_eq!(Ty::List(Box::new(Ty::int())).display(), "[Int]");
    }

    #[test]
    fn display_array() {
        assert_eq!(Ty::Array(Box::new(Ty::int())).display(), "{Int}");
    }

    #[test]
    fn display_dict() {
        assert_eq!(
            Ty::Dict(Box::new(Ty::text()), Box::new(Ty::int())).display(),
            "Dict::(Text, Int)"
        );
    }

    #[test]
    fn display_set() {
        assert_eq!(Ty::Set(Box::new(Ty::int())).display(), "Set::Int");
    }

    #[test]
    fn display_sender() {
        assert_eq!(Ty::Sender(Box::new(Ty::int())).display(), "Sender::Int");
    }

    #[test]
    fn display_receiver() {
        assert_eq!(Ty::Receiver(Box::new(Ty::int())).display(), "Receiver::Int");
    }

    #[test]
    fn display_receiver_factory() {
        assert_eq!(
            Ty::ReceiverFactory(Box::new(Ty::int())).display(),
            "ReceiverFactory::Int"
        );
    }

    #[test]
    fn display_byte() {
        assert_eq!(Ty::Byte.display(), "Byte");
    }

    #[test]
    fn display_bytes() {
        assert_eq!(Ty::Bytes.display(), "Bytes");
    }

    #[test]
    fn display_file() {
        assert_eq!(Ty::File.display(), "File");
    }

    #[test]
    fn display_socket() {
        assert_eq!(Ty::Socket.display(), "Socket");
    }

    #[test]
    fn display_nested_generic() {
        assert_eq!(
            Ty::Generic("Optional".into(), vec![Ty::List(Box::new(Ty::int()))]).display(),
            "Optional::[Int]"
        );
    }

    #[test]
    fn display_to_text_alias() {
        assert_eq!(Ty::int().to_text(), "Int");
        assert_eq!(
            Ty::Function(vec![Ty::int()], Box::new(Ty::text())).to_text(),
            "Lambda(Int -> Text)"
        );
    }
}
