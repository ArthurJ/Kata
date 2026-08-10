//! `TypeEnv` — árvore de escopos para name resolution.
//!
//! Populada no resolution (Pass 0+1) e consumida no inference (Pass 2).
//! Não sobrevive além do typeck — a TAST já carrega os tipos resolvidos.
//!
//! Extraído de `ty.rs` (Passo 6 da zeladoria) — `TypeEnv`/`TypeBinding`
//! formam uma estrutura de dados self-contained, distinta do enum `Ty`.

use std::collections::{HashMap, HashSet};

use super::ty::Ty;

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
    /// Nomes dos parâmetros quando o binding é uma lambda com params nomeados.
    /// `Some(vec!["x", "y"])` para `let f := lambda (x::Int) (y::Int): ...`.
    /// `None` para bindings não-função ou lambdas sem params nomeados.
    /// Usado pelo dict dispatch fallback: quando a DispatchTable não tem
    /// overloads com `param_names`, o type checker consulta o TypeEnv e usa
    /// estes nomes para reordenar o dict.
    pub param_names: Option<Vec<String>>,
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
                param_names: None,
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
                param_names: None,
            },
        );
    }

    /// Define um nome com `param_names` — usado por `let f := lambda ...`
    /// para que o dict dispatch fallback possa acessar os nomes dos params.
    pub fn define_with_param_names(
        &mut self,
        name: &str,
        ty: Ty,
        origin: &str,
        param_names: Vec<String>,
    ) {
        self.bindings.insert(
            name.to_string(),
            TypeBinding {
                ty,
                origin: origin.to_string(),
                fn_alias: None,
                param_names: Some(param_names),
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
                param_names: None,
            },
        );
        self.mutables.insert(name.to_string());
    }

    /// Redefine a origin de um binding existente (ex: __local__ → __module__).
    /// Não muda o tipo nem fn_alias/param_names. No-op se o binding não existe.
    pub fn set_origin(&mut self, name: &str, origin: &str) {
        if let Some(binding) = self.bindings.get_mut(name) {
            binding.origin = origin.to_string();
        }
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

    /// Procura `param_names` de um binding na cadeia de escopos.
    /// Retorna `Some(&[String])` se o binding tem param_names (lambda com
    /// params nomeados), `None` caso contrário.
    pub fn lookup_param_names(&self, name: &str) -> Option<&[String]> {
        if let Some(binding) = self.bindings.get(name) {
            return binding.param_names.as_deref();
        }
        self.parent
            .as_deref()
            .and_then(|p| p.lookup_param_names(name))
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
pub(crate) fn apply_subs_to_ty(ty: &Ty, subs: &HashMap<String, Ty>) -> Ty {
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
