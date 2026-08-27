//! Environment — escopo de variáveis do interpretador.
//!
//! `Vec<HashMap>` para escopo léxico: push em bloco, pop ao sair.
//! `let` faz `scopes.last_mut().insert(name, value)`. `Ident` faz
//! lookup de fora para dentro. `var` é igual — a diferença é que
//! `Reassign` faz `scopes.last_mut().insert(name, new_value)` em
//! vez de falhar.

use std::collections::HashMap;

use crate::value::Value;

pub(crate) struct Env {
    /// Variáveis locais — nome → valor (i64).
    /// Vec<HashMap> para escopo léxico: push em bloco, pop ao sair.
    scopes: Vec<HashMap<String, Value>>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            scopes: vec![HashMap::new()],
        }
    }

    /// Empilha um novo escopo.
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Desempilha o escopo atual.
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Define uma variável no escopo atual.
    pub fn define(&mut self, name: &str, value: Value) {
        self.scopes
            .last_mut()
            .expect("env sem escopos")
            .insert(name.to_string(), value);
    }

    /// Procura uma variável de fora para dentro.
    pub fn lookup(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(&v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    /// Reatribui uma variável existente. Percorre escopos de fora
    /// para dentro. Retorna `Err` se a variável não existe.
    pub fn reassign(&mut self, name: &str, value: Value) -> Result<(), String> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return Ok(());
            }
        }
        Err(format!("variável não declarada: {name}"))
    }
}
