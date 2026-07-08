//! Fundação transversal do compilador Kata.
//!
//! Define os contratos compartilhados entre todas as fases do pipeline:
//! - [`Ty`] — tipo canônico
//! - [`PrimTy`] — mapeamento de representação FFI (não tipo da linguagem)
//! - [`TypeEnv`] — árvore de escopos para name resolution
//! - [`FfiSymbol`] — enum tipado de símbolos FFI
//! - [`TypeShape`] — projeção runtime para reflexão estrutural
//! - [`type_id`] — identificador u32 para a type table do runtime

// Implementação vem no Fio 1.