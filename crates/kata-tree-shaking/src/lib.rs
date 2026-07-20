//! Tree shaking — remove código morto do `TypedModule` antes do codegen AOT.
//!
//! Percorre a TAST a partir do entry point + pre_entry, marcando funções e
//! actions alcançadas. Remove as não alcançadas e todos os `TypedTestSpec`
//! (testes só rodam via `kata test`, nunca em binários de produção).
//!
//! Algoritmo: worklist. Começa com entry + pre_entry. Visita cada expressão
//! recursivamente, coletando referências a funções/actions por nome. Quando
//! uma função/action alcançada é visitada, seus corpos entram na worklist
//! (para descobrir chamadas transitivas). Remove tudo que não foi alcançado.
//!
//! Arestas de alcance:
//! - `Ident{name}` com `expr.ty` = `Ty::Function` → função nomeada (mesma
//!   lógica do codegen em `expr.rs:98` — carrega function pointer)
//! - `Closure{callee: Ident{name}}` com `ffi_symbol=None` → função Kata
//! - `ActionCall{callee}` com `ffi_symbol=None` → Action definida pelo usuário
//! - `Fork{action_name}` → Action (aresta dinâmica por string, per R7)
//!
//! Limitação (DoD): tree shaking só remove funções inteiras. DCE
//! intra-função é post-1.0.

use std::collections::HashSet;

use kata_ast::Spanned;
use kata_core::ty::Ty;
use kata_inference::{
    FusedStage, TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedModule,
};

/// Ponto de entrada — aplica tree shaking ao `TypedModule`.
///
/// Remove `TypedTestSpec` de todas as actions (testes não rodam em build AOT)
/// e funções/actions não alcançadas a partir de `entry` + `pre_entry`.
pub fn tree_shake(typed: TypedModule) -> TypedModule {
    tree_shake_impl(typed, false)
}

/// Como `tree_shake`, mas preserva `TypedTestSpec` nas actions alcançadas.
///
/// Usado por `kata test` (JIT): os testes precisam permanecer para que
/// `jit_compile_tests` gere wrappers `__kata_test_*`.
pub fn tree_shake_preserve_tests(typed: TypedModule) -> TypedModule {
    tree_shake_impl(typed, true)
}

fn tree_shake_impl(typed: TypedModule, preserve_tests: bool) -> TypedModule {
    let TypedModule {
        pre_entry,
        entry,
        dispatch_table,
        type_env,
        functions,
        actions,
    } = typed;

    // ── Coleta nomes alcançados a partir do entry + pre_entry ──
    let mut reached_fns: HashSet<String> = HashSet::new();
    let mut reached_actions: HashSet<String> = HashSet::new();

    // Conjunto de nomes de TypedFunction — usado para distinguir ffi_symbol
    // que aponta para função Kata sintetizada (ex: __kata_repr__Pessoa) de
    // ffi_symbol que aponta para FFI externo (ex: kata_rt_print). Só o
    // primeiro precisa ser coletado para tree shaking.
    let fn_names: HashSet<String> = functions.iter().map(|f| f.name.clone()).collect();

    let mut worklist: Vec<&Spanned<TypedExpr>> = pre_entry.iter().collect();
    worklist.push(&entry);

    // Primeira passada: coleta direto do entry/pre_entry.
    for expr in &worklist {
        collect_refs(&expr.node, &mut reached_fns, &mut reached_actions, &fn_names);
    }

    // Passada transitiva: visita corpos das funções/actions alcançadas
    // até fixpoint (sem novas referências).
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot_fns: Vec<String> = reached_fns.iter().cloned().collect();
        let snapshot_actions: Vec<String> = reached_actions.iter().cloned().collect();

        for name in &snapshot_fns {
            if let Some(func) = functions.iter().find(|f| &f.name == name) {
                for clause in &func.clauses {
                    collect_refs(&clause.body.node, &mut reached_fns, &mut reached_actions, &fn_names);
                    for guard in &clause.guards {
                        if let Some(cond) = &guard.condition {
                            collect_refs(&cond.node, &mut reached_fns, &mut reached_actions, &fn_names);
                        }
                        collect_refs(&guard.body.node, &mut reached_fns, &mut reached_actions, &fn_names);
                    }
                    for wb in &clause.with_bindings {
                        collect_refs(&wb.value.node, &mut reached_fns, &mut reached_actions, &fn_names);
                    }
                }
            }
        }
        for name in &snapshot_actions {
            if let Some(action) = actions.iter().find(|a| &a.name == name) {
                for stmt in &action.body {
                    collect_refs(&stmt.node, &mut reached_fns, &mut reached_actions, &fn_names);
                }
            }
        }

        // Detecta se algo novo foi alcançado comparando com snapshot.
        if reached_fns.len() > snapshot_fns.len() || reached_actions.len() > snapshot_actions.len()
        {
            changed = true;
        }
    }

    // ── Filtra functions e actions ──
    let kept_functions: Vec<TypedFunction> = functions
        .into_iter()
        .filter(|f| reached_fns.contains(&f.name))
        .collect();

    // Actions: remove test specs de TODAS (testes não rodam em AOT),
    // e remove actions não alcançadas.
    let kept_actions: Vec<TypedAction> = actions
        .into_iter()
        .filter(|a| reached_actions.contains(&a.name))
        .map(|mut a| {
            if !preserve_tests {
                a.tests.clear();
            }
            a
        })
        .collect();

    TypedModule {
        pre_entry,
        entry,
        dispatch_table,
        type_env,
        functions: kept_functions,
        actions: kept_actions,
    }
}

/// Percorre um `TypedExpr` recursivamente coletando referências a funções
/// e actions por nome.
///
/// Arestas:
/// - `Ident{name}` com `expr.ty = Ty::Function(...)` → função `name`
/// - `Closure{callee: Ident{name}, ffi_symbol: None}` → função `name`
/// - `Closure{ffi_symbol: Some(sym)}` onde `sym` é nome de TypedFunction → `sym`
///   (função Kata sintetizada, ex: `__kata_repr__Pessoa`)
/// - `ActionCall{callee, ffi_symbol: None}` → action `callee`
/// - `Fork{action_name, ..}` → action `action_name` (aresta dinâmica)
fn collect_refs(
    expr: &TypedExpr,
    reached_fns: &mut HashSet<String>,
    reached_actions: &mut HashSet<String>,
    fn_names: &HashSet<String>,
) {
    match &expr.kind {
        TypedExprKind::Ident { name } => {
            // Caminho do codegen: se ty é Function, é referência a função nomeada.
            if matches!(&expr.ty, Ty::Function(..)) {
                reached_fns.insert(name.clone());
            }
        }

        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
            ..
        } => {
            // Closure não-FFI com callee Ident → função Kata.
            if ffi_symbol.is_none()
                && let TypedExprKind::Ident { name } = &callee.node.kind
            {
                reached_fns.insert(name.clone());
            }
            // Closure com ffi_symbol: Some(sym) onde sym é nome de TypedFunction
            // → função Kata sintetizada (ex: __kata_repr__Pessoa). O codegen
            // procura em kata_refs primeiro, depois em ffi_refs. Coletar sym
            // garante que funções sintetizadas não sejam removidas pelo tree
            // shaking. FFI puro (ex: kata_rt_print) não está em fn_names e é
            // resolvido via ffi_refs, que o tree shaking não toca.
            if let Some(sym) = ffi_symbol {
                if fn_names.contains(sym) {
                    reached_fns.insert(sym.clone());
                }
            }
            // Recursão nos argumentos.
            for arg in args {
                collect_refs(&arg.node, reached_fns, reached_actions, fn_names);
            }
            // Recursão no callee (pode ser sub-expressão em call_indirect).
            collect_refs(&callee.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::ActionCall {
            callee,
            ffi_symbol,
            args,
            ..
        } => {
            // ActionCall não-FFI → Action definida pelo usuário.
            if ffi_symbol.is_none() {
                reached_actions.insert(callee.clone());
            }
            // Recursão nos args (tupla).
            collect_refs(&args.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::Fork {
            action_name, args, ..
        } => {
            // Aresta dinâmica — string match em action_name.
            reached_actions.insert(action_name.clone());
            collect_refs(&args.node, reached_fns, reached_actions, fn_names);
        }

        // ── Sub-expressões — recursão ──
        TypedExprKind::TypeAscription { expr, .. }
        | TypedExprKind::Grouping { inner: expr }
        | TypedExprKind::Return(expr) => collect_refs(&expr.node, reached_fns, reached_actions, fn_names),

        TypedExprKind::Let { value, .. }
        | TypedExprKind::Var { value, .. }
        | TypedExprKind::Reassign { value, .. } => {
            collect_refs(&value.node, reached_fns, reached_actions, fn_names)
        }

        TypedExprKind::Tuple { elements }
        | TypedExprKind::ListLit { elements }
        | TypedExprKind::ArrayLit { elements }
        | TypedExprKind::StructConstruct {
            values: elements, ..
        } => {
            for el in elements {
                collect_refs(&el.node, reached_fns, reached_actions, fn_names);
            }
        }

        TypedExprKind::FieldAccess { expr, .. } | TypedExprKind::IndexAccess { expr, .. } => {
            collect_refs(&expr.node, reached_fns, reached_actions, fn_names)
        }

        TypedExprKind::VariantConstruct { payload, .. } => {
            collect_refs(&payload.node, reached_fns, reached_actions, fn_names)
        }

        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                collect_refs(&clause.body.node, reached_fns, reached_actions, fn_names);
                for guard in &clause.guards {
                    if let Some(cond) = &guard.condition {
                        collect_refs(&cond.node, reached_fns, reached_actions, fn_names);
                    }
                    collect_refs(&guard.body.node, reached_fns, reached_actions, fn_names);
                }
                for wb in &clause.with_bindings {
                    collect_refs(&wb.value.node, reached_fns, reached_actions, fn_names);
                }
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            collect_refs(&scrutinee.node, reached_fns, reached_actions, fn_names);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_refs(&guard.node, reached_fns, reached_actions, fn_names);
                }
                collect_refs(&arm.body.node, reached_fns, reached_actions, fn_names);
            }
        }

        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_refs(&stmt.node, reached_fns, reached_actions, fn_names);
            }
        }

        TypedExprKind::ForIn { iterable, body, .. } => {
            collect_refs(&iterable.node, reached_fns, reached_actions, fn_names);
            for stmt in body {
                collect_refs(&stmt.node, reached_fns, reached_actions, fn_names);
            }
        }

        TypedExprKind::In { item, collection } => {
            collect_refs(&item.node, reached_fns, reached_actions, fn_names);
            collect_refs(&collection.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            collect_refs(&start.node, reached_fns, reached_actions, fn_names);
            collect_refs(&step.node, reached_fns, reached_actions, fn_names);
            collect_refs(&end.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::Map {
            callback,
            collection,
            ..
        }
        | TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => {
            collect_refs(&callback.node, reached_fns, reached_actions, fn_names);
            collect_refs(&collection.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            collect_refs(&callback.node, reached_fns, reached_actions, fn_names);
            collect_refs(&initial.node, reached_fns, reached_actions, fn_names);
            collect_refs(&collection.node, reached_fns, reached_actions, fn_names);
        }

        TypedExprKind::FusedStream { stages, source, .. } => {
            collect_refs(&source.node, reached_fns, reached_actions, fn_names);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                collect_refs(&cb.node, reached_fns, reached_actions, fn_names);
            }
        }

        TypedExprKind::ChannelSend { channel, value } => {
            collect_refs(&channel.node, reached_fns, reached_actions, fn_names);
            collect_refs(&value.node, reached_fns, reached_actions, fn_names);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            collect_refs(&channel.node, reached_fns, reached_actions, fn_names);
        }
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                collect_refs(&arm.channel.node, reached_fns, reached_actions, fn_names);
                collect_refs(&arm.body.node, reached_fns, reached_actions, fn_names);
            }
            if let Some(tm) = timeout_ms {
                collect_refs(&tm.node, reached_fns, reached_actions, fn_names);
            }
            if let Some(tb) = timeout_body {
                collect_refs(&tb.node, reached_fns, reached_actions, fn_names);
            }
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            collect_refs(&factory.node, reached_fns, reached_actions, fn_names)
        }

        // ── Folhas — sem sub-expressões ──
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue
        | TypedExprKind::ChannelCreate { .. } => {}
    }
}
