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
mod constant_fold;
mod ctx;
mod error;
mod fold;
mod jit;
mod predicates;
mod pureza;
mod replace;
mod result;
mod snapshot;
mod walk;

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_core::EnumRegistry;
use kata_inference::{TypedExpr, TypedExprKind, TypedModule};

use ctx::ModuleCtx;
use constness::is_comptime_available;
use fold::fold_literal_calls;
use jit::jit_execute_expr;
use predicates::validate_pending_predicates;
use pureza::check_purity;
use replace::replace_comptime_in_place;
use result::result_to_literal;
use walk::contains_comptime;

// Re-export da API pública.
pub use error::ComptimeError;

/// Serializa um valor JIT-executado em `HeapSnapshotData`.
///
/// Wrapper público sobre `snapshot::serialize_snapshot` (que é `pub(crate)`).
/// Usado pelo REPL para congelar bindings complexos (List, Struct, Tuple,
/// Text, Sum) como snapshots persistidos na root_arena.
pub fn serialize_value(
    raw: i64,
    ty: &kata_core::ty::Ty,
    struct_registry: &kata_core::StructRegistry,
    enum_registry: &kata_core::EnumRegistry,
) -> Result<kata_core::snapshot::HeapSnapshotData, String> {
    snapshot::serialize_snapshot(raw, ty, struct_registry, enum_registry)
}

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
    let mut comptime_bindings: HashMap<String, TypedExpr> = HashMap::new();

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

        // ── Fase 2: Avaliar constants (ConstantBinding) ──
        // Percorre a coleção `constants` do TypedModule. Para cada
        // ConstantBinding, verifica constness do value, JIT-executa,
        // e substitui por literal (escalar) ou HeapSnapshot (complexo).
        // Se não é comptime-available → erro (PRD §3.1).
        // Se o value é uma Lambda → erro específico (PRD §3.7 — Function
        // não é serializável). Peeling Grouping/TypeAscription para
        // detectar lambda envolvida.
        for binding in &mut current.constants {
            let (name, value_span, value_clone) = match &binding.node.kind {
                TypedExprKind::ConstantBinding { name, value } => {
                    (name.clone(), value.span, value.node.clone())
                }
                _ => continue,
            };

            // Pular constants cujo value já foi avaliado (literal ou
            // HeapSnapshot). Isto previne o fixpoint loop: sem o skip,
            // HeapSnapshot é comptime-available → re-avalia → loop infinito.
            // Mas ainda precisamos registrar no comptime_bindings para que
            // fold_constant_refs possa substituir Ident(name) nos corpos de
            // functions e actions (Fase 3).
            if is_already_evaluated(&value_clone) {
                comptime_bindings.insert(name, value_clone);
                continue;
            }

            // Pular Closures (chamadas de função) — o fold_literal_calls
            // cuida de Closures com args literais (ex: `f 41` onde f é
            // named function e 41 é literal). O passo de constants só
            // avalia expressões estruturais (ListLit, StructConstruct,
            // etc.), não chamadas de função.
            if matches!(value_clone.kind, TypedExprKind::Closure { .. }) {
                continue;
            }

            // Detectar lambda como value direto de constant (peeling
            // Grouping e TypeAscription — `(lambda ...)::(Int -> Int)`
            // é o padrão dos testes bidirectional).
            if let Some(ty) = peel_to_lambda_ty(&value_clone) {
                let sig = format_function_sig(&ty);
                return Err(ComptimeError::ConstantLambda {
                    name: name.clone(),
                    sig,
                });
            }

            if !is_comptime_available(&value_clone, &comptime_bindings) {
                return Err(ComptimeError::NotConsttime {
                    reason: format!(
                        "constant {name} — expressão depende de valor runtime"
                    ),
                });
            }
            check_purity(&value_clone)?;
            let result = jit_execute_expr(&value_clone, &ctx, &comptime_bindings)?;
            let literal = result_to_literal(
                &result,
                &value_clone,
                &mut snapshots,
                ctx.struct_registry,
                ctx.enum_registry,
            )?;
            // Substituir o value do ConstantBinding pelo literal.
            if let TypedExprKind::ConstantBinding { value, .. } = &mut binding.node.kind {
                *value = Box::new(Spanned::new(literal.clone(), value_span));
            }
            comptime_bindings.insert(name, literal);
            changed = true;
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
        // Fold em constants: percorre o value de cada ConstantBinding.
        // Isto pega `constant result := f 41` onde f é named function
        // e 41 é literal — fold executa f(41) em compile-time.
        for binding in &mut current.constants {
            if let TypedExprKind::ConstantBinding { value, .. } = &mut binding.node.kind {
                fold_literal_calls(
                    &mut value.node,
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

    // ── Fase 3: Substituir refs a constants nos corpos de functions e actions ──
    // Após o fixpoint, comptime_bindings contém os valores avaliados de todas
    // as constants. Functions e actions compilam em FunctionBuilders separados
    // que não têm acesso ao var_map do entry point (onde constants são
    // registradas). Esta passagem substitui Ident(name) pelo literal/snapshot
    // quando name é uma constant e não está mascarado por um binding local.
    constant_fold::fold_constant_refs_in_functions(
        &mut current.functions,
        &comptime_bindings,
    )?;
    constant_fold::fold_constant_refs_in_actions(
        &mut current.actions,
        &comptime_bindings,
    )?;

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

/// Verifica se o value de um ConstantBinding já foi avaliado (é literal
/// ou HeapSnapshot). Usado para pular re-avaliação no fixpoint loop.
fn is_already_evaluated(expr: &TypedExpr) -> bool {
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

/// Faz peel de Grouping e TypeAscription para verificar se o value
/// subjacente é uma Lambda. Se for, retorna o tipo (Function) da lambda
/// para construir a mensagem de erro com a assinatura esperada.
fn peel_to_lambda_ty(expr: &TypedExpr) -> Option<kata_core::ty::Ty> {
    match &expr.kind {
        TypedExprKind::Lambda { .. } => Some(expr.ty.clone()),
        TypedExprKind::Grouping { inner } => peel_to_lambda_ty(&inner.node),
        TypedExprKind::TypeAscription { expr: inner, .. } => peel_to_lambda_ty(&inner.node),
        _ => None,
    }
}

/// Formata um `Ty::Function` como assinatura Kata (`Int Int => Int`).
fn format_function_sig(ty: &kata_core::ty::Ty) -> String {
    if let kata_core::ty::Ty::Function(params, ret) = ty {
        let params_str = params
            .iter()
            .map(|p| p.display())
            .collect::<Vec<_>>()
            .join(" ");
        format!("{params_str} => {}", ret.display())
    } else {
        ty.display()
    }
}
