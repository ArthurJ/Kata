//! Conversão de `ComptimeResult` (i64 bruto + Ty) em `TypedExpr` literal
//! ou `HeapSnapshot`.

use kata_core::EnumRegistry;
use kata_core::StructRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};

use crate::ctx::ComptimeResult;
use crate::error::ComptimeError;

/// Resolve um `Ty::Struct` que é alias de primitivo até o tipo base.
/// Se `ty` é `Ty::Struct("Altura")` e `Altura` tem `alias_of: "Float"`,
/// retorna `Ty::Prim(Float)`. Para structs não-alias, retorna `None`.
pub(crate) fn resolve_alias_base(ty: &Ty, struct_registry: &StructRegistry) -> Option<Ty> {
    if let Ty::Struct(name) = ty {
        let mut current = name.clone();
        loop {
            let info = struct_registry.get(&current)?;
            let base = info.alias_of.as_ref()?;
            match base.as_str() {
                "Int" => return Some(Ty::Prim(PrimTy::Int)),
                "Float" => return Some(Ty::Prim(PrimTy::Float)),
                "Text" => return Some(Ty::Prim(PrimTy::Text)),
                "Rational" => return Some(Ty::Prim(PrimTy::Rational)),
                _ => {
                    // Alias de outro struct — seguir a cadeia.
                    current = base.clone();
                }
            }
        }
    }
    None
}

/// Converte um `ComptimeResult` (i64 bruto + Ty) num `TypedExpr` literal.
///
/// Fase 1: escalares (Int SMI, Float, Boolean, Unit) → literais directo na TAST.
/// Fase 2: tipos complexos (List, Tuple, Struct, Text, Sum com payload) →
/// `HeapSnapshot` via `serialize_snapshot`.
pub(crate) fn result_to_literal(
    result: &ComptimeResult,
    original: &TypedExpr,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    struct_registry: &StructRegistry,
    enum_registry: &EnumRegistry,
) -> Result<TypedExpr, ComptimeError> {
    // Se result.ty é alias de primitivo (ex: Altura → Float), resolver
    // para o tipo base e produzir o literal correspondente. O alias é
    // transparente em runtime — o valor bruto é o mesmo do tipo base.
    let effective_ty =
        resolve_alias_base(&result.ty, struct_registry).unwrap_or_else(|| result.ty.clone());

    match &effective_ty {
        // ── Escalares: literais directo na TAST ──
        Ty::Prim(PrimTy::Int) => {
            // O valor raw é o valor Kata bruto (SMI-tagged se Int).
            // SMI: LSB=1 → value = (val - 1) >> 1. BigInt: LSB=0 → ponteiro.
            // Para comptime Fase 1, apenas SMIs são suportados (valores
            // pequenos o suficiente para caber em i63). BigInts exigiriam
            // deref do ponteiro no runtime, o que fica para Fase 2.
            let decoded = if (result.raw as u64) & 1 == 1 {
                // SMI
                (result.raw - 1) >> 1
            } else {
                // BigInt — não suportado em Fase 1.
                return Err(ComptimeError::UnsupportedType {
                    ty: result.ty.clone(),
                });
            };
            let text = format!("{}", decoded);
            Ok(TypedExpr {
                span: original.span,
                ty: effective_ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape,
                kind: TypedExprKind::IntLit { text },
            })
        }
        Ty::Prim(PrimTy::Float) => {
            // Float: raw é f64 reinterpretado como i64.
            let f = f64::from_bits(result.raw as u64);
            let text = format!("{}", f);
            Ok(TypedExpr {
                span: original.span,
                ty: effective_ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape,
                kind: TypedExprKind::FloatLit { text },
            })
        }
        Ty::Unit => Ok(TypedExpr {
            span: original.span,
            ty: Ty::Unit,
            tail_pos: original.tail_pos,
            escape: original.escape,
            kind: TypedExprKind::Unit,
        }),
        Ty::Sum(name) if name == "Boolean" => {
            // Boolean::True ou Boolean::False.
            // No runtime, Boolean é representado como i64 (SMI 1 = True, SMI 0 = False).
            let is_true = result.raw != 0;
            Ok(TypedExpr {
                span: original.span,
                ty: result.ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape,
                kind: TypedExprKind::VariantQual {
                    enum_name: "Boolean".into(),
                    variant: if is_true { "True" } else { "False" }.into(),
                    tag: if is_true { 0 } else { 1 },
                    module_path: None,
                },
            })
        }
        // ── Tipos complexos: serializar em HeapSnapshot ──
        Ty::List(_)
        | Ty::Tuple(_)
        | Ty::Struct(_)
        | Ty::Prim(PrimTy::Text)
        | Ty::Sum(_)
        | Ty::Generic(_, _)
        | Ty::Function(_, _) => {
            let snapshot = crate::snapshot::serialize_snapshot(
                result.raw,
                &result.ty,
                struct_registry,
                enum_registry,
            )
            .map_err(|e| ComptimeError::JitError {
                reason: format!("serialização de snapshot: {e}"),
            })?;
            let snapshot_id = snapshots.len() as u32;
            snapshots.push(snapshot);
            Ok(TypedExpr {
                span: original.span,
                ty: result.ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape,
                kind: TypedExprKind::HeapSnapshot {
                    snapshot_id,
                    ty: result.ty.clone(),
                },
            })
        }
        other => Err(ComptimeError::UnsupportedType { ty: other.clone() }),
    }
}
