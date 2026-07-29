//! Comptime pass — avaliação em compile-time via JIT-and-execute.
//!
//! Percorre a TAST identificando nós `TypedExprKind::Comptime`, verifica
//! constness e pureza, JIT-executa a expressão, e substitui por `Literal`
//! (escalares) ou `HeapSnapshot` (tipos complexos — Fase 2).
//!
//! Posição no pipeline:
//! ```text
//! ... → tree_shake → comptime → lowering → ...
//! ```
//!
//! O pass recebe um `TypedModule` e retorna um `TypedModule` com os nós
//! `Comptime` substituídos. Para a Fase 1, apenas resultados escalares
//! (Int, Float, Boolean, Unit) são suportados — viram `IntLit`/`FloatLit`/
//! `TextLit`/`Unit` directo na TAST.

mod constness;
mod pureza;
mod snapshot;

use kata_ast::Spanned;
use kata_core::StructRegistry;
use kata_core::dispatch::DispatchTable;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_inference::{TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedModule};

use constness::is_comptime_available;
use pureza::check_purity;

/// Dados imutáveis do módulo necessários para constness e JIT execution.
///
/// Referências aos campos individuais de `TypedModule` — Rust permite borrow
/// de campos diferentes do mesmo struct simultaneamente (partial borrow),
/// evitando o conflito entre `&mut current.pre_entry`/`&mut current.entry`
/// e `&current.dispatch_table` etc.
struct ModuleCtx<'a> {
    dispatch_table: &'a DispatchTable,
    type_env: &'a TypeEnv,
    functions: &'a [TypedFunction],
    actions: &'a [TypedAction],
    struct_registry: &'a StructRegistry,
}

/// Resultado da execução comptime — valor bruto + tipo.
struct ComptimeResult {
    raw: i64,
    ty: Ty,
}

/// Erro do comptime pass.
#[derive(Debug)]
pub enum ComptimeError {
    /// Expressão não é comptime-available (depende de valor runtime).
    NotConsttime { reason: String },
    /// Expressão é impura (contém ActionCall, Fork, etc.).
    Impure { reason: String },
    /// Erro durante JIT execution.
    JitError { reason: String },
    /// Tipo de resultado não suportado nesta fase.
    UnsupportedType { ty: Ty },
}

impl std::fmt::Display for ComptimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComptimeError::NotConsttime { reason } => {
                write!(f, "não é comptime-available: {reason}")
            }
            ComptimeError::Impure { reason } => {
                write!(f, "expressão impura: {reason}")
            }
            ComptimeError::JitError { reason } => {
                write!(f, "erro de JIT: {reason}")
            }
            ComptimeError::UnsupportedType { ty } => {
                write!(f, "tipo não suportado em comptime (Fase 1): {ty}")
            }
        }
    }
}

impl std::error::Error for ComptimeError {}

/// Executa o comptime pass num `TypedModule`.
///
/// Percorre `pre_entry` e `entry` substituindo nós `TypedExprKind::Comptime`
/// por literais (escalares) ou snapshots (complexos — Fase 2).
///
/// Repete até fixpoint (sem novos nós `Comptime`).
pub fn run_comptime_pass(typed: TypedModule) -> Result<TypedModule, ComptimeError> {
    let mut current = typed;
    // Acumulador de snapshots — populado por replace_comptime_in_place.
    // No fim, atribuído a current.snapshots.
    let mut snapshots: Vec<kata_core::snapshot::HeapSnapshotData> =
        std::mem::take(&mut current.snapshots);

    // Fixpoint: substituir Comptime pode revelar novos Comptime em inner exprs.
    loop {
        let mut changed = false;

        // Partial borrow: empresta campos imutáveis individuais de `current`.
        // Não conflita com `&mut current.pre_entry` / `&mut current.entry`
        // porque são campos diferentes do mesmo struct.
        let ctx = ModuleCtx {
            dispatch_table: &current.dispatch_table,
            type_env: &current.type_env,
            functions: &current.functions,
            actions: &current.actions,
            struct_registry: &current.struct_registry,
        };

        // Processar pre_entry
        for expr in &mut current.pre_entry {
            let was_comptime = contains_comptime(&expr.node);
            replace_comptime_in_place(&mut expr.node, &ctx, &mut changed, &mut snapshots)?;
            if was_comptime && !contains_comptime(&expr.node) {
                // Já substituído — ok
            }
        }

        // Processar entry
        replace_comptime_in_place(&mut current.entry.node, &ctx, &mut changed, &mut snapshots)?;

        if !changed {
            break;
        }
    }

    current.snapshots = snapshots;
    Ok(current)
}

/// Substitui nós `Comptime` recursivamente num `TypedExpr`.
fn replace_comptime_in_place(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    changed: &mut bool,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
) -> Result<(), ComptimeError> {
    if !matches!(expr.kind, TypedExprKind::Comptime { .. }) {
        // Recursão nos filhos.
        walk_mut(expr, &mut |child| {
            replace_comptime_in_place(child, ctx, changed, snapshots)
        })?;
        return Ok(());
    }

    // Extrair o inner do Comptime via mem::replace para evitar borrow conflict.
    // Usa Unit como placeholder (será sobrescrito).
    let inner_owned = match std::mem::replace(&mut expr.kind, TypedExprKind::Unit) {
        TypedExprKind::Comptime { expr } => *expr,
        _ => unreachable!(),
    };
    let inner = &inner_owned.node;

    // Caso especial: @comptime envolve um Let. Avaliar apenas o
    // `value` do let e preservar o binding, substituindo o Comptime
    // inteiro por `Let { name, value: <literal> }`.
    if let TypedExprKind::Let { name, value } = &inner.kind {
        // 1. Verificar constness do value (não do let inteiro).
        if !is_comptime_available(&value.node) {
            // Restaurar antes de propagar erro.
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(ComptimeError::NotConsttime {
                reason: "expressão depende de valor runtime".into(),
            });
        }

        // 2. Verificar pureza do value.
        if let Err(e) = check_purity(&value.node) {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }

        // 3. JIT-executar o value.
        let result = match jit_execute_expr(&value.node, ctx) {
            Ok(r) => r,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 4. Substituir por literal ou HeapSnapshot.
        let literal = match result_to_literal(&result, &value.node, snapshots, ctx.struct_registry)
        {
            Ok(l) => l,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 5. Reconstruir o Let com o value substituído pelo literal.
        expr.kind = TypedExprKind::Let {
            name: name.clone(),
            value: Box::new(Spanned::new(literal, value.span)),
        };
        expr.ty = inner.ty.clone();
        *changed = true;
        return Ok(());
    }

    // Caso geral: @comptime envolve uma expressão qualquer.
    // 1. Verificar constness do inner expr.
    if !is_comptime_available(inner) {
        expr.kind = TypedExprKind::Comptime {
            expr: Box::new(inner_owned),
        };
        return Err(ComptimeError::NotConsttime {
            reason: "expressão depende de valor runtime".into(),
        });
    }

    // 2. Verificar pureza.
    if let Err(e) = check_purity(inner) {
        expr.kind = TypedExprKind::Comptime {
            expr: Box::new(inner_owned),
        };
        return Err(e);
    }

    // 3. JIT-executar o inner expr.
    let result = match jit_execute_expr(inner, ctx) {
        Ok(r) => r,
        Err(e) => {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }
    };

    // 4. Substituir por literal (escalar) ou HeapSnapshot (complexo).
    let replacement = match result_to_literal(&result, inner, snapshots, ctx.struct_registry) {
        Ok(r) => r,
        Err(e) => {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }
    };

    // 5. Trocar o kind do expr.
    expr.kind = replacement.kind;
    expr.ty = replacement.ty;
    *changed = true;
    Ok(())
}

/// Walk mut nos filhos de `expr` (não no próprio expr).
fn walk_mut<F>(expr: &mut TypedExpr, f: &mut F) -> Result<(), ComptimeError>
where
    F: FnMut(&mut TypedExpr) -> Result<(), ComptimeError>,
{
    match &mut expr.kind {
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            f(&mut value.node)?;
        }
        TypedExprKind::LetDestruct {
            value, bindings, ..
        } => {
            f(&mut value.node)?;
            for (_, b) in bindings.iter_mut() {
                f(&mut b.node)?;
            }
        }
        TypedExprKind::Closure { callee, args, .. } => {
            f(&mut callee.node)?;
            for arg in args.iter_mut() {
                f(&mut arg.node)?;
            }
        }
        TypedExprKind::Grouping { inner } => f(&mut inner.node)?,
        TypedExprKind::Tuple { elements } => {
            for el in elements.iter_mut() {
                f(&mut el.node)?;
            }
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for v in values.iter_mut() {
                f(&mut v.node)?;
            }
        }
        TypedExprKind::FieldAccess { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::IndexAccess { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::TypeAscription { expr, .. } => f(&mut expr.node)?,
        TypedExprKind::TypeOf { expr } => f(&mut expr.node)?,
        TypedExprKind::Comptime { expr } => f(&mut expr.node)?,
        TypedExprKind::Match { scrutinee, arms } => {
            f(&mut scrutinee.node)?;
            for arm in arms.iter_mut() {
                if let Some(guard) = &mut arm.guard {
                    f(&mut guard.node)?;
                }
                f(&mut arm.body.node)?;
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses.iter_mut() {
                for guard in &mut clause.guards {
                    if let Some(cond) = &mut guard.condition {
                        f(&mut cond.node)?;
                    }
                    f(&mut guard.body.node)?;
                }
                for wb in &mut clause.with_bindings {
                    f(&mut wb.value.node)?;
                }
                if clause.guards.is_empty() {
                    f(&mut clause.body.node)?;
                }
            }
        }
        TypedExprKind::Return(inner) => f(&mut inner.node)?,
        TypedExprKind::Reassign { value, .. } => f(&mut value.node)?,
        TypedExprKind::Loop { body } => {
            for stmt in body.iter_mut() {
                f(&mut stmt.node)?;
            }
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements.iter_mut() {
                f(&mut el.node)?;
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            f(&mut start.node)?;
            f(&mut step.node)?;
            f(&mut end.node)?;
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            f(&mut iterable.node)?;
            for stmt in body.iter_mut() {
                f(&mut stmt.node)?;
            }
        }
        // HeapSnapshot — folha.
        TypedExprKind::HeapSnapshot { .. } => {}
        // Outros variants não têm filhos TypedExpr ou não aparecem em top-level.
        _ => {}
    }
    Ok(())
}

/// Verifica se a expressão contém algum nó `Comptime`.
fn contains_comptime(expr: &TypedExpr) -> bool {
    let mut found = false;
    walk_ref(expr, &mut |e| {
        if matches!(e.kind, TypedExprKind::Comptime { .. }) {
            found = true;
        }
    });
    found
}

/// Walk imutável nos filhos de `expr`.
fn walk_ref<F: FnMut(&TypedExpr)>(expr: &TypedExpr, f: &mut F) {
    f(expr);
    match &expr.kind {
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            walk_ref(&value.node, f);
        }
        TypedExprKind::Comptime { expr } => walk_ref(&expr.node, f),
        TypedExprKind::Closure { callee, args, .. } => {
            walk_ref(&callee.node, f);
            for arg in args {
                walk_ref(&arg.node, f);
            }
        }
        TypedExprKind::Grouping { inner } => walk_ref(&inner.node, f),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                walk_ref(&el.node, f);
            }
        }
        TypedExprKind::HeapSnapshot { .. } => {}
        _ => {}
    }
}

/// Converte um `ComptimeResult` (i64 bruto + Ty) num `TypedExpr` literal.
///
/// Fase 1: escalares (Int SMI, Float, Boolean, Unit) → literais directo na TAST.
/// Fase 2: tipos complexos (List, Tuple, Struct, Text, Sum com payload) →
/// `HeapSnapshot` via `serialize_snapshot`.
fn result_to_literal(
    result: &ComptimeResult,
    original: &TypedExpr,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    struct_registry: &StructRegistry,
) -> Result<TypedExpr, ComptimeError> {
    match &result.ty {
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
                ty: result.ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape.clone(),
                kind: TypedExprKind::IntLit { text },
            })
        }
        Ty::Prim(PrimTy::Float) => {
            // Float: raw é f64 reinterpretado como i64.
            let f = f64::from_bits(result.raw as u64);
            let text = format!("{}", f);
            Ok(TypedExpr {
                span: original.span,
                ty: result.ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape.clone(),
                kind: TypedExprKind::FloatLit { text },
            })
        }
        Ty::Unit => Ok(TypedExpr {
            span: original.span,
            ty: Ty::Unit,
            tail_pos: original.tail_pos,
            escape: original.escape.clone(),
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
                escape: original.escape.clone(),
                kind: TypedExprKind::VariantQual {
                    enum_name: "Boolean".into(),
                    variant: if is_true { "True" } else { "False" }.into(),
                    tag: if is_true { 0 } else { 1 },
                    module_path: None,
                },
            })
        }
        // ── Tipos complexos: serializar em HeapSnapshot ──
        Ty::List(_) | Ty::Tuple(_) | Ty::Struct(_) | Ty::Prim(PrimTy::Text) | Ty::Sum(_) => {
            let snapshot = snapshot::serialize_snapshot(result.raw, &result.ty, struct_registry)
                .map_err(|e| ComptimeError::JitError {
                    reason: format!("serialização de snapshot: {e}"),
                })?;
            let snapshot_id = snapshots.len() as u32;
            snapshots.push(snapshot);
            Ok(TypedExpr {
                span: original.span,
                ty: result.ty.clone(),
                tail_pos: original.tail_pos,
                escape: original.escape.clone(),
                kind: TypedExprKind::HeapSnapshot {
                    snapshot_id,
                    ty: result.ty.clone(),
                },
            })
        }
        other => Err(ComptimeError::UnsupportedType { ty: other.clone() }),
    }
}

/// JIT-executa uma expressão TAST.
///
/// Cria um `TypedModule` mínimo com a expressão como entry point,
/// chama `jit_eval`, e retorna o resultado bruto.
fn jit_execute_expr(
    expr: &TypedExpr,
    ctx: &ModuleCtx<'_>,
) -> Result<ComptimeResult, ComptimeError> {
    // Criar um mini TypedModule com apenas a expressão como entry.
    let mini = TypedModule {
        pre_entry: Vec::new(),
        entry: Spanned::new(expr.clone(), expr.span),
        dispatch_table: ctx.dispatch_table.clone(),
        type_env: ctx.type_env.clone(),
        functions: ctx.functions.to_vec(),
        actions: ctx.actions.to_vec(),
        struct_registry: ctx.struct_registry.clone(),
        snapshots: Vec::new(),
    };

    let result = kata_codegen::jit_eval(&mini).map_err(|e| ComptimeError::JitError {
        reason: format!("{e}"),
    })?;

    Ok(ComptimeResult {
        raw: result.raw,
        ty: result.ty,
    })
}
