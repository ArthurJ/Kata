//! `DispatchTable` — tabela de overloads com despacho por dominância.
//!
//! Algoritmo (nasce em Fio 1, mesmo com 1 overload):
//! 1. FILTRAR: candidatas compatíveis (arity + match_score)
//! 2. PONTUAR: match_score → Score(exact, alias, refined, iface)
//! 3. ORDENAR: lexicográfico decrescente (exact, alias, refined, iface, !generic)
//! 4. TOPO ÚNICO → Ok
//! 5. EMPATE → AmbiguousDispatch
//!
//! Em Fio 1: só `exact` é não-zero. Alias (Fio 5), refined (Fio 6),
//! iface (Fio 7) são sempre 0. Mas a estrutura do algoritmo está pronta.

use crate::ty::Ty;
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
    pub fn insert_ffi(
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
    pub fn resolve(&self, name: &str, args: &[Ty]) -> Result<OverloadInfo, DispatchError> {
        self.resolve_inner(name, args, false)
    }

    fn resolve_inner(
        &self,
        name: &str,
        args: &[Ty],
        tried_commutative: bool,
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

            let score = match_score(args, &info.params);

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
            return self.resolve_inner(name, &swapped, true);
        }

        if candidates.is_empty() {
            // Tenta dar mensagem útil
            if self.has_function(name) {
                if let Some(first) = overloads.first() {
                    return Err(DispatchError::TypeMismatch {
                        name: name.to_string(),
                        expected: format!("{:?}", first.params),
                        found: format!("{:?}", args),
                    });
                }
            }
            return Err(DispatchError::FunctionNotFound {
                name: name.to_string(),
                arg_count: args.len(),
            });
        }

        // Ordenar por score decrescente (lexicográfico)
        candidates.sort_by(|a, b| b.score.cmp(&a.score));

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
    pub fn iter_entries(&self) -> impl Iterator<Item = (&String, &Vec<OverloadInfo>)> {
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
/// Em Fio 1: só `exact` existe. Alias (Fio 5), refined (Fio 6),
/// iface (Fio 7) são sempre 0. Se qualquer posição é incompatível,
/// retorna Score::incompatible() (todos zero).
fn match_score(args: &[Ty], params: &[Ty]) -> Score {
    let mut exact = 0;
    let mut alias = 0;
    let mut refined = 0;
    let mut iface = 0;

    for (arg, param) in args.iter().zip(params) {
        if arg == param {
            exact += 1;
        } else {
            // Fio 1: só exact match conta. Se não é exato, é incompatível.
            // Fios posteriores adicionarão: alias, refined, iface aqui.
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
