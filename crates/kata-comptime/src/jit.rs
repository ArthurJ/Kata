//! JIT execution de uma expressão TAST via `kata_codegen::jit_eval`.

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_inference::{TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedModule};

use crate::ctx::{ComptimeResult, ModuleCtx};
use crate::error::ComptimeError;

/// JIT-executa uma expressão TAST.
///
/// Cria um `TypedModule` mínimo com a expressão como entry point,
/// chama `jit_eval`, e retorna o resultado bruto.
///
/// `comptime_bindings` é injetado como pre_entry (Let bindings) no mini
/// TypedModule para que o JIT resolva Idents comptime-available.
///
/// `functions` e `actions` controlam quais funções/actions nomeadas são
/// incluídas no mini módulo. O codegen compila eageramente TODAS as
/// funções e actions no TypedModule — incluir funções/actions com refs
/// a symbols ausentes causa erros de codegen. Para avaliar constants
/// estruturais (ListLit, etc.), passar `&[]` (nenhuma necessária). Para
/// fold de chamadas literais, passar todas (catch_unwind captura falhas).
///
/// `snapshots` são os snapshots do comptime pass. O mini módulo precisa
/// carregar estes snapshots na root_arena para que `HeapSnapshot` nodes
/// na expressão ou nos corpos das funções resolvam via
/// `kata_rt_get_snapshot`.
pub(crate) fn jit_execute_expr(
    expr: &TypedExpr,
    ctx: &ModuleCtx<'_>,
    comptime_bindings: &HashMap<String, TypedExpr>,
    functions: &[TypedFunction],
    actions: &[TypedAction],
    snapshots: &[kata_core::snapshot::HeapSnapshotData],
) -> Result<ComptimeResult, ComptimeError> {
    // Construir pre_entry com os bindings comptime-available.
    // Cada binding vira um `Let { name, value: literal }` em pre_entry.
    let mut pre_entry = Vec::new();
    for (name, value) in comptime_bindings {
        let span = value.span;
        pre_entry.push(Spanned::new(
            TypedExpr {
                span,
                ty: value.ty.clone(),
                tail_pos: false,
                escape: value.escape,
                kind: TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(value.clone(), span)),
                },
            },
            span,
        ));
    }

    // Criar um mini TypedModule com os bindings em pre_entry e a
    // expressão como entry. Os snapshots do comptime pass são passados
    // intactos — os snapshot_ids na TAST já referenciam índices neste Vec.
    let mini = TypedModule {
        pre_entry,
        entry: Spanned::new(expr.clone(), expr.span),
        dispatch_table: ctx.dispatch_table.clone(),
        type_env: ctx.type_env.clone(),
        functions: functions.to_vec(),
        actions: actions.to_vec(),
        struct_registry: ctx.struct_registry.clone(),
        snapshots: snapshots.to_vec(),
        refined_decls: Vec::new(),
        constants: Vec::new(),
    };

    let result =
        kata_codegen::jit_eval(&mini, &Default::default(), &[], kata_codegen::leak_rt_ptr())
            .map_err(|e| ComptimeError::JitError {
                reason: format!("{e}"),
            })?;

    Ok(ComptimeResult {
        raw: result.raw,
        ty: result.ty,
    })
}