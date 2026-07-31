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
use kata_core::EnumRegistry;
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
    enum_registry: &'a EnumRegistry,
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
pub fn run_comptime_pass(
    typed: TypedModule,
    enum_registry: &EnumRegistry,
) -> Result<TypedModule, ComptimeError> {
    let mut current = typed;
    // Acumulador de snapshots — populado por replace_comptime_in_place.
    // No fim, atribuído a current.snapshots.
    let mut snapshots: Vec<kata_core::snapshot::HeapSnapshotData> =
        std::mem::take(&mut current.snapshots);

    // Bindings comptime-available — construído incrementalmente durante o
    // fixpoint. Após substituir @comptime let x := ..., x → literal é
    // adicionado aqui. Um @comptime posterior que referencia x vê o binding
    // no mapa. O mapa também é injetado no mini TypedModule para o JIT
    // resolver Idents comptime-available.
    let mut comptime_bindings: std::collections::HashMap<String, TypedExpr> =
        std::collections::HashMap::new();

    // Fixpoint: substituir Comptime pode revelar novos Comptime em inner exprs.
    loop {
        let mut changed = false;

        // Clonar actions antes do loop para evitar conflito de borrow:
        // o ctx precisa de &actions (imutável) para jit_execute_expr, mas
        // precisamos mutar current.actions[i].body. Clonar resolve
        // (consistente com jit_execute_expr que já clona tudo).
        let actions_clone = current.actions.clone();

        // Partial borrow: empresta campos imutáveis individuais de `current`.
        // Não conflita com `&mut current.pre_entry` / `&mut current.entry`
        // porque são campos diferentes do mesmo struct.
        let ctx = ModuleCtx {
            dispatch_table: &current.dispatch_table,
            type_env: &current.type_env,
            functions: &current.functions,
            actions: &actions_clone,
            struct_registry: &current.struct_registry,
            enum_registry,
        };

        // Processar pre_entry
        for expr in &mut current.pre_entry {
            let was_comptime = contains_comptime(&expr.node);
            replace_comptime_in_place(
                &mut expr.node,
                &ctx,
                &mut changed,
                &mut snapshots,
                &mut comptime_bindings,
            )?;
            if was_comptime && !contains_comptime(&expr.node) {
                // Já substituído — ok
            }
        }

        // Processar entry
        replace_comptime_in_place(
            &mut current.entry.node,
            &ctx,
            &mut changed,
            &mut snapshots,
            &mut comptime_bindings,
        )?;

        // Processar bodies de actions (Fase 3b)
        for action in &mut current.actions {
            for stmt in &mut action.body {
                replace_comptime_in_place(
                    &mut stmt.node,
                    &ctx,
                    &mut changed,
                    &mut snapshots,
                    &mut comptime_bindings,
                )?;
            }
        }

        // ── Ponto 7: Constant folding de chamadas com args literais ──
        // Após replace_comptime_in_place, percorre a TAST procurando
        // Closures com ffi_symbol: None e todos args literais. JIT-executa
        // e substitui por literal. O fixpoint garante que folds em cascade
        // (o resultado de um fold pode ser arg literal de outro).
        for expr in &mut current.pre_entry {
            fold_literal_calls(
                &mut expr.node,
                &ctx,
                &mut changed,
                &mut snapshots,
                &comptime_bindings,
            )?;
        }
        fold_literal_calls(
            &mut current.entry.node,
            &ctx,
            &mut changed,
            &mut snapshots,
            &comptime_bindings,
        )?;
        for action in &mut current.actions {
            for stmt in &mut action.body {
                fold_literal_calls(
                    &mut stmt.node,
                    &ctx,
                    &mut changed,
                    &mut snapshots,
                    &comptime_bindings,
                )?;
            }
        }

        if !changed {
            break;
        }
    }

    current.snapshots = snapshots;

    // ── Fase 4: Validar predicados complexos pendentes ──
    // TypeAscription com pending_predicates foi produzida pelo typeck quando
    // const_eval não conseguiu avaliar (predicado complexo, ex: is_prime).
    // Aqui, JIT-executamos cada predicado e verificamos se retorna Boolean::True.
    let actions_clone = current.actions.clone();
    let ctx = ModuleCtx {
        dispatch_table: &current.dispatch_table,
        type_env: &current.type_env,
        functions: &current.functions,
        actions: &actions_clone,
        struct_registry: &current.struct_registry,
        enum_registry,
    };
    for expr in &mut current.pre_entry {
        validate_pending_predicates(&mut expr.node, &ctx, &comptime_bindings)?;
    }
    validate_pending_predicates(&mut current.entry.node, &ctx, &comptime_bindings)?;
    for action in &mut current.actions {
        for stmt in &mut action.body {
            validate_pending_predicates(&mut stmt.node, &ctx, &comptime_bindings)?;
        }
    }
    Ok(current)
}

/// Substitui nós `Comptime` recursivamente num `TypedExpr`.
fn replace_comptime_in_place(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    changed: &mut bool,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    comptime_bindings: &mut std::collections::HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    if !matches!(expr.kind, TypedExprKind::Comptime { .. }) {
        // Recursão nos filhos.
        walk_mut(expr, &mut |child| {
            replace_comptime_in_place(child, ctx, changed, snapshots, comptime_bindings)
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
        if !is_comptime_available(&value.node, comptime_bindings) {
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
        let result = match jit_execute_expr(&value.node, ctx, comptime_bindings) {
            Ok(r) => r,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 4. Substituir por literal ou HeapSnapshot.
        let literal = match result_to_literal(
            &result,
            &value.node,
            snapshots,
            ctx.struct_registry,
            ctx.enum_registry,
        ) {
            Ok(l) => l,
            Err(e) => {
                expr.kind = TypedExprKind::Comptime {
                    expr: Box::new(inner_owned),
                };
                return Err(e);
            }
        };

        // 5. Reconstruir o Let com o value substituído pelo literal.
        let literal_expr = Spanned::new(literal.clone(), value.span);
        expr.kind = TypedExprKind::Let {
            name: name.clone(),
            value: Box::new(literal_expr),
        };
        expr.ty = inner.ty.clone();
        // 6. Registrar o binding como comptime-available para dataflow.
        comptime_bindings.insert(name.clone(), literal);
        *changed = true;
        return Ok(());
    }

    // Caso geral: @comptime envolve uma expressão qualquer.
    // 1. Verificar constness do inner expr.
    if !is_comptime_available(inner, comptime_bindings) {
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
    let result = match jit_execute_expr(inner, ctx, comptime_bindings) {
        Ok(r) => r,
        Err(e) => {
            expr.kind = TypedExprKind::Comptime {
                expr: Box::new(inner_owned),
            };
            return Err(e);
        }
    };

    // 4. Substituir por literal (escalar) ou HeapSnapshot (complexo).
    let replacement = match result_to_literal(
        &result,
        inner,
        snapshots,
        ctx.struct_registry,
        ctx.enum_registry,
    ) {
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
/// Resolve um `Ty::Struct` que é alias de primitivo até o tipo base.
/// Se `ty` é `Ty::Struct("Altura")` e `Altura` tem `alias_of: "Float"`,
/// retorna `Ty::Prim(Float)`. Para structs não-alias, retorna `None`.
fn resolve_alias_base(ty: &Ty, struct_registry: &StructRegistry) -> Option<Ty> {
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

fn result_to_literal(
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
        | Ty::Generic(_, _) => {
            let snapshot = snapshot::serialize_snapshot(
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

/// Walk recursivo nos filhos de `expr` chamando `validate_pending_predicates`.
/// Quando encontra `TypeAscription` com `pending_predicates` não-vazio,
/// JIT-executa cada predicado e verifica se retorna `Boolean::True`.
fn validate_pending_predicates(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    comptime_bindings: &std::collections::HashMap<String, TypedExpr>,
) -> Result<(), ComptimeError> {
    // Primeiro recursar nos filhos.
    walk_mut(expr, &mut |child| {
        validate_pending_predicates(child, ctx, comptime_bindings)
    })?;

    // Depois processar o próprio nó se for TypeAscription com pending.
    if let TypedExprKind::TypeAscription {
        pending_predicates, ..
    } = &mut expr.kind
        && !pending_predicates.is_empty()
    {
        for pred in pending_predicates.iter() {
            let result = jit_execute_expr(&pred.node, ctx, comptime_bindings)?;
            // Resultado deve ser Boolean::True (tag 1) ou Boolean::False (tag 0).
            // O runtime representa Boolean como Sum com tag 0 (False) ou 1 (True).
            if result.raw != 1 {
                return Err(ComptimeError::JitError {
                    reason: format!(
                        "predicado de ascription refined falhou: \
                         esperava Boolean::True, obteve tag {}",
                        result.raw
                    ),
                });
            }
        }
        // Todos os predicados passaram — limpa pending.
        pending_predicates.clear();
    }
    Ok(())
}

/// JIT-executa uma expressão TAST.
///
/// Cria um `TypedModule` mínimo com a expressão como entry point,
/// chama `jit_eval`, e retorna o resultado bruto.
///
/// `comptime_bindings` é injetado como pre_entry (Let bindings) no mini
/// TypedModule para que o JIT resolva Idents comptime-available.
fn jit_execute_expr(
    expr: &TypedExpr,
    ctx: &ModuleCtx<'_>,
    comptime_bindings: &std::collections::HashMap<String, TypedExpr>,
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
    // expressão como entry.
    let mini = TypedModule {
        pre_entry,
        entry: Spanned::new(expr.clone(), expr.span),
        dispatch_table: ctx.dispatch_table.clone(),
        type_env: ctx.type_env.clone(),
        functions: ctx.functions.to_vec(),
        actions: ctx.actions.to_vec(),
        struct_registry: ctx.struct_registry.clone(),
        snapshots: Vec::new(),
        refined_decls: Vec::new(),
    };

    let result = kata_codegen::jit_eval(&mini, &Default::default()).map_err(|e| {
        ComptimeError::JitError {
            reason: format!("{e}"),
        }
    })?;

    Ok(ComptimeResult {
        raw: result.raw,
        ty: result.ty,
    })
}

// ── Ponto 7: Constant folding de chamadas com args literais ──

/// Verifica se um `TypedExpr` é um literal "puro" — literal que não
/// depende de execução e pode ser usado como argumento de fold.
///
/// Aceitos: IntLit, FloatLit, TextLit, Unit, HeapSnapshot, VariantQual
/// (variant sem payload — Boolean::True, Result::Err sem payload, etc.).
/// Não aceitos: VariantConstruct (tem payload — precisa avaliar o payload),
/// Closure, Ident, etc.
fn is_literal_expr(expr: &TypedExpr) -> bool {
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
fn fold_literal_calls(
    expr: &mut TypedExpr,
    ctx: &ModuleCtx<'_>,
    changed: &mut bool,
    snapshots: &mut Vec<kata_core::snapshot::HeapSnapshotData>,
    comptime_bindings: &std::collections::HashMap<String, TypedExpr>,
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
