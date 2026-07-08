//! `MetadataTable` — sidecar read-only com metadados do lowering.
//!
//! Populado durante o lowering TAST→CLIF, consultado por passes futuros
//! (ARC pass, comptime, debugger). Em Fio 1, `closure_info` e `escape_flags`
//! são vazios — existem para evitar retrofit.

use std::collections::HashMap;

use cranelift_codegen::ir::{Block, Inst, Value};
use kata_core::ty::Ty;

/// Origem de uma instrução CLIF — mapeia para o nó da TAST que a gerou.
#[derive(Debug, Clone, Copy)]
pub struct InstOrigin {
    /// Índice da expressão na TAST (profundidade-first order).
    pub expr_idx: usize,
}

/// Origem de um bloco CLIF.
#[derive(Debug, Clone, Copy)]
pub struct BlockOrigin {
    /// Rótulo semântico do bloco (ex: "entry", "let_merge").
    pub label: &'static str,
}

/// Tipo de um valor CLIF mapeado para `Ty` do compilador.
#[derive(Debug, Clone)]
pub struct ValueMeta {
    /// Tipo canônico do valor.
    pub ty: Ty,
    /// Se o valor é resultado de chamada FFI.
    pub from_ffi: bool,
}

/// Metadados de closure — vazio em Fio 1 (closures são Fio 9).
#[derive(Debug, Clone, Default)]
pub struct ClosureInfo {
    /// Nome da closure (se nomeada).
    pub name: Option<String>,
    /// Se a closure escapa (captura variável do escopo externo).
    pub escapes: bool,
}

/// Flags de escape analysis — vazio em Fio 1 (TRMA é Fio 11).
#[derive(Debug, Clone, Default)]
pub struct EscapeFlags {
    /// Se o valor alocado no heap escapa do escopo atual.
    pub escapes: bool,
}

/// Tabela de metadados sidecar — read-only após o lowering.
///
/// Cada campo é indexado por `Inst`/`Block`/`Value` do Cranelift.
/// O codegen popula durante o lowering; passes futuros consultam.
#[derive(Debug, Clone, Default)]
pub struct MetadataTable {
    /// Origem de cada instrução CLIF.
    pub inst_origins: HashMap<Inst, InstOrigin>,
    /// Origem de cada bloco CLIF.
    pub block_origins: HashMap<Block, BlockOrigin>,
    /// Metadados de tipo por valor CLIF.
    pub value_types: HashMap<Value, ValueMeta>,
    /// Informações de closure (vazio em Fio 1).
    pub closure_info: HashMap<String, ClosureInfo>,
    /// Flags de escape (vazio em Fio 1).
    pub escape_flags: HashMap<Value, EscapeFlags>,
}

impl MetadataTable {
    /// Cria uma tabela vazia.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra a origem de uma instrução.
    pub fn record_inst(&mut self, inst: Inst, origin: InstOrigin) {
        self.inst_origins.insert(inst, origin);
    }

    /// Registra a origem de um bloco.
    pub fn record_block(&mut self, block: Block, origin: BlockOrigin) {
        self.block_origins.insert(block, origin);
    }

    /// Registra metadados de um valor.
    pub fn record_value(&mut self, value: Value, meta: ValueMeta) {
        self.value_types.insert(value, meta);
    }
}
