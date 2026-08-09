//! `DispatchTable` — tabela de overloads com despacho por dominância.
//!
//! Algoritmo (nasce, mesmo com 1 overload):
//! 1. FILTRAR: candidatas compatíveis (arity + match_score)
//! 2. PONTUAR: match_score → Score(exact, alias, refined, iface)
//! 3. ORDENAR: lexicográfico decrescente (exact, alias, refined, iface, !generic)
//! 4. TOPO ÚNICO → Ok
//! 5. EMPATE → AmbiguousDispatch
//!
//! Só `exact` é não-zero. Alias, refined,
//! iface são sempre 0. Mas a estrutura do algoritmo está pronta.

use crate::interface_registry::InterfaceRegistry;
use crate::ty::{Ty, ty_list_to_string};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

/// Informação de uma sobrecarga registrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverloadInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub ffi_symbol: Option<String>,
    pub is_action: bool,
    pub is_generic: bool,
    pub is_constructor: bool,
    pub associative_neutral: Option<i64>,
    /// Nomes dos type params (ex: `["T"]` para `id :: T => T`).
    /// Vazio para funções não-genéricas.
    pub type_params: Vec<String>,
    /// Instanciação nos call sites. `None` para função genérica
    /// original e para funções não-genéricas. `Some(map)` para instâncias
    /// monomorfizadas.
    pub substitutions: Option<HashMap<String, Ty>>,
    /// Nomes dos params da action. `Some(nome)` para params nomeados,
    /// `None` para posicional legado. Vazio para funções puras e FFI.
    /// Usado pelo typeck para mapear DictLit args → params nomeados.
    pub param_names: Vec<Option<String>>,
}

/// Score de um candidato — 4D + tiebreak genérico.
///
/// Ordenação lexicográfica: mais exact vence, depois mais alias,
/// depois mais refined, depois mais iface. Concreto vence genérico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
    pub exact: usize,
    pub alias: usize,
    pub refined: usize,
    pub iface: usize,
    /// false (concreto) vence true (genérico) no tiebreak.
    pub is_generic_origin: bool,
}

impl Score {
    /// Score incompatível — descarta o candidato.
    pub fn incompatible() -> Self {
        Score {
            exact: 0,
            alias: 0,
            refined: 0,
            iface: 0,
            is_generic_origin: false,
        }
    }

    /// Verifica se o score é compatível (todos args deram match em alguma categoria).
    pub fn is_compatible(&self, arg_count: usize) -> bool {
        self.exact + self.alias + self.refined + self.iface == arg_count
    }
}

/// Candidato a dispatch: overload + score.
#[derive(Debug, Clone)]
struct Candidate {
    info: OverloadInfo,
    score: Score,
}

/// Resultado de dispatch parcial — args ausentes (holes) recebem tipos do overload.
#[derive(Debug, Clone)]
pub struct PartialDispatchResult {
    /// O overload que casou.
    pub overload: OverloadInfo,
    /// Tipos esperados nas posições ausentes (holes).
    /// `Some(ty)` na posição i = a posição i era ausente (hole) e o tipo esperado é `ty`.
    /// `None` na posição i = a posição i era presente (não é hole).
    pub hole_types: Vec<Option<Ty>>,
}

/// Tabela de dispatch indexada por nome.
#[derive(Debug, Clone)]
pub struct DispatchTable {
    entries: HashMap<String, Vec<OverloadInfo>>,
    commutative: HashSet<String>,
}

impl Default for DispatchTable {
    fn default() -> Self {
        Self::new()
    }
}

impl DispatchTable {
    pub fn new() -> Self {
        DispatchTable {
            entries: HashMap::new(),
            commutative: HashSet::new(),
        }
    }

    /// Registra uma sobrecarga.
    pub fn insert(&mut self, info: OverloadInfo) {
        let overloads = self.entries.entry(info.name.clone()).or_default();
        if !overloads.contains(&info) {
            overloads.push(info);
        }
    }

    /// Registra uma função FFI (conveniência).
    #[allow(dead_code)]
    pub(crate) fn insert_ffi(
        &mut self,
        name: &str,
        params: Vec<Ty>,
        ret: Ty,
        ffi_symbol: String,
        associative_neutral: Option<i64>,
    ) {
        self.insert(OverloadInfo {
            name: name.to_string(),
            params,
            ret,
            ffi_symbol: Some(ffi_symbol),
            is_action: false,
            is_generic: false,
            is_constructor: false,
            associative_neutral,
            type_params: vec![],
            substitutions: None,
            param_names: vec![],
        });
    }

    /// Marca função como comutativa (para dispatch tentar args invertidos).
    pub fn mark_commutative(&mut self, name: &str) {
        self.commutative.insert(name.to_string());
    }

    /// Verifica se uma função existe.
    pub fn has_function(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Retorna todas as overloads de um nome.
    pub fn get_overloads(&self, name: &str) -> Option<&Vec<OverloadInfo>> {
        self.entries.get(name)
    }

    /// Resolve uma chamada: nome + tipos dos argumentos → OverloadInfo.
    ///
    /// Algoritmo: coletar candidatos por nome, pontuar por compatibilidade,
    /// selecionar o de maior score. Empate → AmbiguousDispatch.
    pub fn resolve(
        &self,
        name: &str,
        args: &[Ty],
        iface_reg: &InterfaceRegistry,
    ) -> Result<OverloadInfo, DispatchError> {
        self.resolve_inner(name, args, false, iface_reg)
    }

    /// Resolve uma chamada parcial: alguns args são `None` (holes ausentes).
    ///
    /// Holes (`None`) não participam do scoring — não somam nem excluem.
    /// Se exatamente um overload casa com os args presentes, retorna os tipos
    /// esperados nas posições ausentes. Se múltiplos casam, é ambíguo.
    pub fn resolve_partial(
        &self,
        name: &str,
        args: &[Option<Ty>],
        iface_reg: &InterfaceRegistry,
    ) -> Result<PartialDispatchResult, DispatchError> {
        self.resolve_partial_inner(name, args, false, iface_reg)
    }

    fn resolve_partial_inner(
        &self,
        name: &str,
        args: &[Option<Ty>],
        tried_commutative: bool,
        iface_reg: &InterfaceRegistry,
    ) -> Result<PartialDispatchResult, DispatchError> {
        let overloads = self
            .entries
            .get(name)
            .ok_or(DispatchError::FunctionNotFound {
                name: name.to_string(),
                arg_count: args.len(),
            })?;

        let mut candidates: Vec<(OverloadInfo, Vec<Option<Ty>>)> = Vec::new();

        for info in overloads {
            // Arity mismatch → skip
            if info.params.len() != args.len() {
                continue;
            }

            // Pontua apenas args presentes. None = não restringe.
            let mut compatible = true;
            for (arg_opt, param) in args.iter().zip(&info.params) {
                if let Some(arg_ty) = arg_opt
                    && arg_ty != param
                {
                    // Verifica iface match.
                    let iface_match = match extract_iface_name(param) {
                        Some(iface_name) => match extract_type_name(arg_ty) {
                            Some(type_name) => iface_reg.type_implements(&type_name, &iface_name),
                            None => false,
                        },
                        None => false,
                    };
                    if !iface_match {
                        compatible = false;
                        break;
                    }
                }
                // None (hole) — não pontua, não exclui
            }

            if compatible {
                // Constrói hole_types: Some(param_ty) onde arg era None, None onde era Some.
                let hole_types: Vec<Option<Ty>> = args
                    .iter()
                    .zip(&info.params)
                    .map(|(arg_opt, param)| {
                        if arg_opt.is_none() {
                            Some(param.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                candidates.push((info.clone(), hole_types));
            }
        }

        // Commutative short-circuit: se 0 candidatos e função é comutativa, tenta invertida
        if candidates.is_empty()
            && !tried_commutative
            && self.commutative.contains(name)
            && args.len() == 2
        {
            let swapped = vec![args[1].clone(), args[0].clone()];
            return self.resolve_partial_inner(name, &swapped, true, iface_reg);
        }

        if candidates.is_empty() {
            if self.has_function(name)
                && let Some(first) = overloads.first()
            {
                return Err(DispatchError::TypeMismatch {
                    name: name.to_string(),
                    expected: ty_list_to_string(&first.params),
                    found: args
                        .iter()
                        .map(|a| {
                            a.as_ref()
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| "?".into())
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
            return Err(DispatchError::FunctionNotFound {
                name: name.to_string(),
                arg_count: args.len(),
            });
        }

        // Comutativo pode gerar múltiplos candidatos via swap — pegamos o primeiro.
        // Para dispatch parcial, se múltiplos overloads diferentes casam (não via
        // comutatividade), é ambíguo. Verificamos se há overloads distintos.
        let unique_overloads: Vec<&OverloadInfo> =
            candidates.iter().map(|(info, _)| info).collect();
        let first_params = &unique_overloads[0].params;
        let all_same = unique_overloads.iter().all(|oi| oi.params == *first_params);

        if all_same {
            // Todos casam com o mesmo overload (ex: comutatividade duplicou) — retorna o primeiro
            return Ok(PartialDispatchResult {
                overload: candidates[0].0.clone(),
                hole_types: candidates[0].1.clone(),
            });
        }

        // Múltiplos overloads distintos casam → ambíguo
        Err(DispatchError::AmbiguousDispatch {
            name: name.to_string(),
            arg_count: args.len(),
        })
    }

    fn resolve_inner(
        &self,
        name: &str,
        args: &[Ty],
        tried_commutative: bool,
        iface_reg: &InterfaceRegistry,
    ) -> Result<OverloadInfo, DispatchError> {
        let overloads = self
            .entries
            .get(name)
            .ok_or(DispatchError::FunctionNotFound {
                name: name.to_string(),
                arg_count: args.len(),
            })?;

        let mut candidates: Vec<Candidate> = Vec::new();

        for info in overloads {
            // Arity mismatch → skip
            if info.params.len() != args.len() {
                continue;
            }

            let score = match_score(args, &info.params, iface_reg);

            if score.is_compatible(args.len()) {
                candidates.push(Candidate {
                    info: info.clone(),
                    score: Score {
                        is_generic_origin: info.is_generic,
                        ..score
                    },
                });
            }
        }

        // Commutative short-circuit: se 0 candidatos e função é comutativa, tenta invertida
        if candidates.is_empty()
            && !tried_commutative
            && self.commutative.contains(name)
            && args.len() == 2
        {
            let swapped = vec![args[1].clone(), args[0].clone()];
            return self.resolve_inner(name, &swapped, true, iface_reg);
        }

        if candidates.is_empty() {
            // Tenta dar mensagem útil
            if self.has_function(name)
                && let Some(first) = overloads.first()
            {
                return Err(DispatchError::TypeMismatch {
                    name: name.to_string(),
                    expected: ty_list_to_string(&first.params),
                    found: ty_list_to_string(args),
                });
            }
            return Err(DispatchError::FunctionNotFound {
                name: name.to_string(),
                arg_count: args.len(),
            });
        }

        // Ordenar por score decrescente (lexicográfico)
        candidates.sort_by_key(|b| Reverse(b.score));

        let best = &candidates[0];
        let best_score = best.score;

        let top_count = candidates.iter().filter(|c| c.score == best_score).count();

        if top_count == 1 {
            return Ok(best.info.clone());
        }

        // Empate → AmbiguousDispatch
        Err(DispatchError::AmbiguousDispatch {
            name: name.to_string(),
            arg_count: args.len(),
        })
    }

    /// Itera sobre todas as entradas (para debug/inspeção).
    #[allow(dead_code)]
    pub(crate) fn iter_entries(&self) -> impl Iterator<Item = (&String, &Vec<OverloadInfo>)> {
        self.entries.iter()
    }
}

/// Erro de dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    FunctionNotFound {
        name: String,
        arg_count: usize,
    },
    TypeMismatch {
        name: String,
        expected: String,
        found: String,
    },
    AmbiguousDispatch {
        name: String,
        arg_count: usize,
    },
}

/// Conta matches por categoria: exact, alias, refined, iface.
///
/// - `exact`: arg == param (já existe desde o início)
/// - `iface`: param é `Ty::Interface(name)` e arg implementa essa interface
///   via `InterfaceRegistry::type_implements`
///
/// Alias e refined ainda são sempre 0 — serão populados
/// em fios posteriores. Se qualquer posição é incompatível, retorna
/// `Score::incompatible()` (todos zero).
pub fn match_score(args: &[Ty], params: &[Ty], iface_reg: &InterfaceRegistry) -> Score {
    let mut exact = 0;
    let alias = 0;
    let refined = 0;
    let mut iface = 0;

    for (arg, param) in args.iter().zip(params) {
        if arg == param {
            exact += 1;
        } else if let Some(iface_name) = extract_iface_name(param)
            && let Some(type_name) = extract_type_name(arg)
            && iface_reg.type_implements(&type_name, &iface_name)
        {
            iface += 1;
        } else if let Ty::OverloadSet { overloads, .. } = arg
            && let Ty::Action(param_params, param_ret) = param
        {
            // OverloadSet vs Action: algum overload do OverloadSet é compatível
            // com o tipo Action(params, ret) esperado pelo parâmetro?
            let matched = overloads.iter().any(|(ov_params, ov_ret)| {
                if ov_params.len() != param_params.len() {
                    return false;
                }
                let score = match_score(param_params, ov_params, iface_reg);
                score.is_compatible(param_params.len()) && *ov_ret == **param_ret
            });
            if matched {
                iface += 1;
            } else {
                return Score::incompatible();
            }
        } else {
            // Não é exato, não é alias, não é refined, não é iface → incompatível.
            return Score::incompatible();
        }
    }

    Score {
        exact,
        alias,
        refined,
        iface,
        is_generic_origin: false,
    }
}

/// Extrai o nome de interface de um `Ty` se for `Ty::Interface` ou
/// `Ty::Generic` cujo nome é uma interface registrada.
/// `Ty::Interface("NUM")` → `"NUM"`
/// `Ty::Generic("ITERABLE", [A])` → `"ITERABLE"` (interface parametrizada)
fn extract_iface_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Interface(name) => Some(name.clone()),
        Ty::Generic(name, _) => Some(name.clone()),
        _ => None,
    }
}

/// Extrai o nome do tipo concreto de um `Ty` para consulta no InterfaceRegistry.
/// `Ty::Prim(PrimTy::Int)` → `"Int"`
/// `Ty::Struct("Complex")` → `"Complex"`
/// `Ty::Sum("Boolean")` → `"Boolean"`
/// `Ty::Generic("List", [Int])` → `"List"`
/// `Ty::List(Int)` → `"List"` (tipo intrínseco)
/// `Ty::Array(Int)` → `"Array"` (tipo intrínseco)
/// `Ty::Range(Int)` → `"Range"` (tipo intrínseco)
fn extract_type_name(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Prim(crate::ty::PrimTy::Int) => Some("Int".into()),
        Ty::Prim(crate::ty::PrimTy::Float) => Some("Float".into()),
        Ty::Prim(crate::ty::PrimTy::Text) => Some("Text".into()),
        Ty::Prim(crate::ty::PrimTy::Rational) => Some("Rational".into()),
        Ty::Struct(name) => Some(name.clone()),
        Ty::Sum(name) => Some(name.clone()),
        Ty::Generic(name, _) => Some(name.clone()),
        Ty::List(_) => Some("List".into()),
        Ty::Array(_) => Some("Array".into()),
        Ty::Range(_) => Some("Range".into()),
        Ty::Dict(_, _) => Some("Dict".into()),
        Ty::Set(_) => Some("Set".into()),
        Ty::Bytes => Some("Bytes".into()),
        Ty::Byte => Some("Byte".into()),
        Ty::Sender(_) => Some("Sender".into()),
        Ty::Receiver(_) => Some("Receiver".into()),
        Ty::ReceiverFactory(_) => Some("ReceiverFactory".into()),
        _ => None,
    }
}
