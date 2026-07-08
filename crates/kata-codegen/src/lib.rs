//! Lowering TAST → CLIF (Cranelift IR) + MetadataTable sidecar + emit.
//!
//! Sem IR intermediária própria — o lowering é direto TAST → CLIF.
//! Block arguments nativos (Cranelift 0.133) — sem stack slots.
//! MetadataTable é read-only após lowering, consultada pelo ARC pass.

// Implementação vem no Fio 1.
