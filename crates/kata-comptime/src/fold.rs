//! Constant folding de chamadas com args literais (Ponto 7).

use std::collections::HashMap;

use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind};

use crate::ctx::ModuleCtx;
use crate::error::ComptimeError;
use crate::jit::jit_execute_expr;
use crate::result::result_to_literal;
use crate::walk::walk_mut;

/// Verifica se um `TypedExpr` é um literal "puro" — literal que não
/// depende de execução e pode ser usado como argumento de fold.
///
/// Aceitos: IntLit, FloatLit, TextLit, Unit, HeapSnapshot, VariantQual
/// (variant sem payload — Boolean::True, Result::Err sem payload, etc.).
/// Não aceitos: VariantConstruct (tem payload — precisa avaliar o payload),
/// Closure, Ident, etc.
pub(crate) fn is_literal_expr(expr: &TypedExpr) -> bool {
    matches!(
        &expr.kind,
        TypedExprKind::IntLit { .. }
            | TypedExprKind::FloatLit { .. }
            | TypedExprKind::TextLit { .. }
            | TypedExprKind::Unit
            | TypedExprKind::HeapSnapshot { .. }
            | TypedExprKind::VariantQual { .. }
    )
}

/// Percorre a TAST procurando `Closure` com `ffi_symbol: None` e todos
/// args literais. JIT-executa a Closure e substitui por literal.
///
/// Usa `walk_mut` para recursão nos filhos primeiro (bottom-up): os filhos
/// podem conter Closures foldable que, ao serem dobradas, transformam um
/// Closure pai (cujo arg era uma Closure) numa Closure com args literais
/// na próxima iteração do fixpoint.
pub(crate) fn fold_literal_calls(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    changed: &mut bool,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    comptime_bindings: &HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    // Recursão bottom-up nos filhos primeiro.
    walk_mut(expr, &mut |child| {
        fold_literal_calls(child, ctx, changed, snapshots, comptime_bindings)
    })?;

    // Após recursão, verificar se o próprio nó é uma Closure foldable.
    if let TypedExprKind::Closure {
        callee,
        args,
        ffi_symbol: None,
    } = &expr.kind
    {
        // Callee deve ser Ident (função Kata pura nomeada) cujo nome
        // existe na lista de TypedFunction do módulo. Isto exclui alias
        // constructors, builtins, e outros constructs que podem ter
        // ffi_symbol: None mas não são funções puras comuns.
        let is_pure_callee = match &callee.node.kind {
            TypedExprKind::Ident { name } => {
                matches!(&callee.node.ty, Ty::Function(..))
                    && ctx.functions.iter().any(|f| f.name == *name)
            }
            // Lambdas anônimas também são puras por design, mas só
            // fazemos fold de lambdas com corpo direto (não recursivo).
            // O JIT executa o lambda como entry point do mini módulo.
            TypedExprKind::Lambda { .. } => true,
            _ => false,
        };

        if is_pure_callee && args.iter().all(|a| is_literal_expr(&a.node)) {
            // Não dobrar construtores falíveis — Closure que retorna
            // Result (ex: Peso 70.0? com predicado refined). O fold via
            // JIT não preserva a semântica do predicado e produz um literal
            // com tipo errado (HeapSnapshot para Result em vez do valor).
            if matches!(&expr.ty, Ty::Generic(name, _) if name == "Result") {
                return Ok(());
            }
            // JIT-executar a Closure inteira.
            // Usar catch_unwind porque o Cranelift pode panicar em vez de
            // retornar Err (ex: type mismatch, alias edge cases). Um panic
            // não deve quebrar a compilação — apenas não faz fold.
            let closure_expr = expr.clone();
            let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                jit_execute_expr(&closure_expr, ctx, comptime_bindings)
            })) {
                Ok(Ok(r)) => r,
                // JIT retornou Err ou panic — não faz fold.
                Ok(Err(_)) | Err(_) => return Ok(()),
            };

            // Substituir por literal ou HeapSnapshot.
            let replacement = match result_to_literal(
                &result,
                &closure_expr,
                snapshots,
                ctx.struct_registry,
                ctx.enum_registry,
            ) {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };

            // Preservar o tipo original da expressão. O JIT vê aliases
            // como seus tipos base (alias é transparente no runtime),
            // mas a TAST precisa manter o tipo original (ex: Altura, não
            // Float) para o codegen não quebrar com type mismatch.
            let original_ty = expr.ty.clone();
            expr.kind = replacement.kind;
            expr.ty = original_ty;
            *changed = true;
        }
    }

    Ok(())
}