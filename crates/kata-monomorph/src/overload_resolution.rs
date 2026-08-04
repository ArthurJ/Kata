//! Resolução de overloads genéricas — extraído de `lib.rs`.
//!
//! Contém as funções que encontram e instanciam overloads genéricas do
//! DispatchTable durante a monomorphização:
//! - `find_generic_overload`: busca por `unify`
//! - `instantiate_generic_closure`: gera instância concreta de um call site
//! - `resolve_erased_ffi_symbol`: resolve ffi_symbol para Closures type-erased

use std::collections::HashMap;

use kata_ast::Spanned;
use kata_core::ty::Ty;
use kata_inference::{Substitutions, TypedExpr, TypedExprKind, apply_subs, unify};

use crate::instantiate::{instantiate_action, instantiate_function};
use crate::naming::canonicalize_subs;
use crate::{MonoCtx, RewriteAcc};

/// Busca a overload genérica que casa com `arg_types` por `unify`.
///
/// Itera por todas as overloads com `type_params` não-vazio e mesma aridade,
/// tentando `unify` em cada uma. Retorna a primeira que unifica com sucesso
/// (junto com as substituições computadas). Isto é necessário porque pode
/// haver múltiplas overloads genéricas com a mesma aridade — ex: `show` tem
/// `show :: Optional<T> => Text` e `show :: Result<T,E> => Text`, ambas com
/// `params.len()==1`. Um `.find()` simples pela aridade pararia na primeira
/// (Optional) mesmo quando os args são `Result`, falhando o `unify` sem tentar
/// a próxima.
pub(crate) fn find_generic_overload<'a>(
    overloads: &'a [kata_core::dispatch::OverloadInfo],
    arg_types: &[Ty],
) -> Option<(&'a kata_core::dispatch::OverloadInfo, Substitutions)> {
    for oi in overloads {
        if oi.type_params.is_empty() || oi.params.len() != arg_types.len() {
            continue;
        }
        let mut subs: Substitutions = HashMap::new();
        if unify(&oi.params, arg_types, &oi.type_params, &mut subs).is_ok() {
            return Some((oi, subs));
        }
    }
    None
}

/// Tenta instanciar uma `Closure` cujo callee é `Ident(name)` com overload
/// genérica que unifica com os tipos dos argumentos.
///
/// Se encontrar uma overload genérica que unifica, gera a instância
/// (OverloadInfo + TypedFunction se o template tem corpo), rewrites o callee
/// para o nome da instância, instancia `callee.ty` com as mesmas substituições,
/// zera `ffi_symbol`, e retorna `true`. Retorna `false` se não há overload
/// genérica casável (call site não-genérico).
///
/// ffi_symbol: se a instância tem corpo Kata (função sintetizada, como
/// `__kata_show__Result`), seta None — o codegen resolve via kata_refs pelo
/// nome da instância. Se é FFI pura (sem corpo, como `head`), mantém o
/// ffi_symbol da template — o codegen resolve via ffi_refs pelo sym_name.
pub(crate) fn instantiate_generic_closure(
    callee: &mut Spanned<TypedExpr>,
    args: &[Spanned<TypedExpr>],
    ffi_symbol: &mut Option<String>,
    name: &str,
    ctx: &MonoCtx,
    acc: &mut RewriteAcc,
) -> bool {
    let Some(overloads) = ctx.dispatch_table.get_overloads(name) else {
        return false;
    };

    // Procura overload genérica que unifica com os arg types.
    // Tenta unify em cada candidata (pode haver múltiplas overloads genéricas
    // com mesma aridade — ex: show para Optional<T> e Result<T,E>).
    let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
    let Some((oi, subs)) = find_generic_overload(overloads, &arg_types) else {
        return false;
    };

    // Guarda: se algum type_param mapeia para Ty::Var(_), a instanciação é
    // trivial (não-concreta). Isto acontece quando uma função genérica
    // template (ex: __kata_show__List) está sendo percorrida pelo
    // monomorphizador e contém chamadas a outras funções genéricas com
    // args ainda não-resolvidos (ex: __kata_show__List_rest t onde
    // t :: List(Var("A"))). Neste caso, não instanciar — a instanciação
    // ocorrerá quando a função template for instanciada para um tipo
    // concreto e o body for reescrito com tipos resolvidos.
    if subs.values().any(|ty| matches!(ty, Ty::Var(_))) {
        return false;
    }

    // Gera nome canônico da instância.
    let subs_key = canonicalize_subs(&oi.type_params, &subs);
    let instance_name = format!("{name}_{subs_key}");

    // Procura a função original (template) pelo nome mangled (ffi_symbol)
    // ou pelo nome direto. Isto determina se a instância tem corpo Kata
    // (função sintetizada) ou é FFI pura (sem corpo).
    let func_lookup_name = oi.ffi_symbol.as_deref().unwrap_or(name);
    let orig_func = ctx.functions.iter().find(|f| f.name == func_lookup_name);

    // Verifica se a instância já existe.
    if !ctx.existing.contains(&instance_name)
        && !acc.new_overloads.iter().any(|o| o.name == instance_name)
    {
        // SEMPRE gera OverloadInfo (entrada no DispatchTable com tipos
        // concretos). Isto cobre o caso de funções genéricas sem corpo
        // (apenas Sig no DispatchTable, como `id :: T => T` sem cláusulas).
        let instance_ffi_symbol = if orig_func.is_some() {
            None
        } else {
            oi.ffi_symbol.clone()
        };
        acc.new_overloads.push(kata_core::dispatch::OverloadInfo {
            name: instance_name.clone(),
            params: oi.params.iter().map(|t| apply_subs(t, &subs)).collect(),
            ret: apply_subs(&oi.ret, &subs),
            ffi_symbol: instance_ffi_symbol,
            is_action: false,
            is_generic: false,
            is_constructor: false,
            associative_neutral: None,
            type_params: vec![],
            substitutions: Some(subs.clone()),
            param_names: vec![],
        });

        // Gera TypedFunction se a função original tem corpo.
        if let Some(orig_func) = orig_func {
            let mono_func = instantiate_function(orig_func, &subs, &instance_name);
            acc.new_functions.push(mono_func);
        }
    }

    // Rewrite o callee para o nome da instância, instancia o callee.ty com
    // as mesmas substituições (garante consistência com param_types da
    // instância em kata_refs), e zera ffi_symbol — o codegen resolve via
    // kata_refs pelo nome do callee (instância).
    callee.node.kind = TypedExprKind::Ident {
        name: instance_name,
    };
    callee.node.ty = apply_subs(&callee.node.ty, &subs);
    // If the overload is FFI-only (no Kata body), keep the ffi_symbol
    // so the codegen can emit a direct FFI call. If it has a body,
    // zero it — the codegen resolves via kata_refs by instance name.
    if orig_func.is_none() {
        *ffi_symbol = oi.ffi_symbol.clone();
    } else {
        *ffi_symbol = None;
    }
    true
}

/// Resolução de ffi_symbol para Closures type-erased (Layer 5).
///
/// Quando uma Action polimórfica por interface é instanciada (ex:
/// `echo_SHOW_Int`), o body contém Closures produzidas por
/// `try_iface_method_dispatch` com `ffi_symbol: None`. Após
/// `instantiate_action` aplicar `apply_subs`, os tipos dos args são concretos
/// (ex: [Int]). Se o DispatchTable tem um overload concreto (não-genérico)
/// que casa e possui `ffi_symbol: Some(...)`, preenchemos aqui.
///
/// Isto resolve `show msg` dentro de `echo_SHOW_Int`: DispatchTable tem
/// `show :: Int => Text @ffi("kata_rt_bi_show")`.
///
/// Fallback gracioso: se o arg_type é `Ty::Var(_)` (type param não resolvido
/// — ex: `E` em `Result::Ok 42`, onde a variante Err nunca é construída),
/// não há overload concreto. O braço do Match nunca executa em runtime, mas o
/// codegen precisa de um nó válido. Substituímos a Closure por `TextLit("?")`
/// após o match (ver `fallback::fallback_unresolved_show`).
pub(crate) fn resolve_erased_ffi_symbol(
    name: &str,
    args: &[Spanned<TypedExpr>],
    ffi_symbol: &mut Option<String>,
    ctx: &MonoCtx,
) {
    if ffi_symbol.is_some() {
        return;
    }
    let arg_types: Vec<Ty> = args.iter().map(|a| a.node.ty.clone()).collect();
    if let Some(overloads) = ctx.dispatch_table.get_overloads(name) {
        let concrete = overloads.iter().find(|oi| {
            oi.type_params.is_empty()
                && oi.params.len() == arg_types.len()
                && oi.params == arg_types
        });
        if let Some(oi) = concrete {
            *ffi_symbol = oi.ffi_symbol.clone();
        }
    }
}

/// Instancia uma Action genérica encontrada em um `ActionCall`.
///
/// Extraído do braço `ActionCall` de `rewrite_typed_expr` para reduzir
/// acoplamento. Gera a instância concreta (OverloadInfo + TypedAction)
/// e reescreve o callee para o nome da instância.
pub(crate) fn instantiate_generic_action_call(
    callee: &mut String,
    args: &Spanned<TypedExpr>,
    ctx: &MonoCtx,
    acc: &mut RewriteAcc,
) {
    let Some(overloads) = ctx.dispatch_table.get_overloads(callee) else {
        return;
    };

    // Procura overload genérico com mesma aridade dos args.
    let arg_types: Vec<Ty> = match &args.node.kind {
        TypedExprKind::Tuple { elements } => elements.iter().map(|e| e.node.ty.clone()).collect(),
        TypedExprKind::Unit => Vec::new(),
        _ => vec![args.node.ty.clone()],
    };
    let generic_overload = find_generic_overload(overloads, &arg_types);

    if let Some((oi, subs)) = generic_overload {
        // Gera nome canônico da instância.
        let subs_key = canonicalize_subs(&oi.type_params, &subs);
        let instance_name = format!("{callee}_{subs_key}");

        // Verifica se a instância já existe.
        if !ctx.existing.contains(&instance_name)
            && !acc.new_overloads.iter().any(|o| o.name == instance_name)
        {
            // Gera OverloadInfo com tipos concretos.
            acc.new_overloads.push(kata_core::dispatch::OverloadInfo {
                name: instance_name.clone(),
                params: oi.params.iter().map(|t| apply_subs(t, &subs)).collect(),
                ret: apply_subs(&oi.ret, &subs),
                ffi_symbol: None,
                is_action: true,
                is_generic: false,
                is_constructor: false,
                associative_neutral: None,
                type_params: vec![],
                substitutions: Some(subs.clone()),
                param_names: vec![],
            });

            // Gera TypedAction se a Action original tem corpo.
            // Casa por nome + aridade: sobrecargas com o mesmo nome (ex: `echo`
            // com 1 vs 2 params) fazem o `find` simples retornar a errada.
            // O `oi` (OverloadInfo selecionado por find_generic_overload) já
            // tem a aridade correta — usar oi.params.len() como filtro.
            if let Some(orig_action) = ctx
                .actions
                .iter()
                .find(|a| a.name == *callee && a.param_types.len() == oi.params.len())
            {
                let mono_action = instantiate_action(orig_action, &subs, &instance_name);
                acc.new_actions.push(mono_action);
            }
        }

        // Rewrite o callee para o nome da instância.
        *callee = instance_name;
    }
}
