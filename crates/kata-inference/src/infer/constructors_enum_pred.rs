//! Construtores despachadores para enums com variantes predicadas.
//!
//! Sintetiza funções predicado e construtor despachador para enums do tipo
//! `enum IMC: Magreza(< _ 18.5), Normal(<= _ 25.0), ...`.
//!
//! Diferença do refined: o construtor retorna `Sum` direto (não `Result`),
//! despachando para a variante cujo predicado satisfaz. A última variante
//! (sem predicado) é o fallback/default.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::{DispatchTable, OverloadInfo};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_resolution::EnumPredDeclInfo;

use crate::desugar;
use crate::typed::{
    TypedExpr, TypedExprKind, TypedFunction, TypedGuardClause, TypedLambdaClause, TypedPattern,
};

use super::constructors_refined::{build_pred_call, substitute_hole};
use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Sintetiza funções predicado e construtor despachador para enums com
/// variantes predicadas (`enum IMC: Magreza(< _ 18.5), Normal(<= _ 25.0), ...`).
///
/// Diferença do refined: o construtor retorna `Sum` direto (não `Result`),
/// despachando para a variante cujo predicado satisfaz. A última variante
/// (sem predicado) é o fallback/default.
pub(crate) fn synthesize_enum_pred(
    enum_pred_decls: &[EnumPredDeclInfo],
    enum_registry: &kata_core::enum_registry::EnumRegistry,
    struct_registry: &kata_core::struct_registry::StructRegistry,
    type_env: &TypeEnv,
    dispatch_table: &mut DispatchTable,
) -> InferResult<Vec<TypedFunction>> {
    if enum_pred_decls.is_empty() {
        return Ok(Vec::new());
    }

    // ── Passo 1: registra assinaturas no DispatchTable ──
    for decl in enum_pred_decls {
        // Registra cada função predicado: __pred_enum_EnumName_Variant :: payload_ty => Boolean
        for variant in &decl.variants {
            if variant.predicate.is_some() {
                let pred_name = format!("__pred_enum_{}_{}", decl.name, variant.name);
                dispatch_table.insert(OverloadInfo {
                    name: pred_name,
                    params: vec![decl.payload_ty.clone()],
                    ret: Ty::boolean(),
                    ffi_symbol: None,
                    is_action: false,
                    is_generic: false,
                    is_constructor: false,
                    associative_neutral: None,
                    type_params: vec![],
                    substitutions: None,
                    param_names: vec![],
                });
            }
        }

        // Registra construtor despachador: EnumName :: payload_ty => EnumName
        dispatch_table.insert(OverloadInfo {
            name: decl.name.clone(),
            params: vec![decl.payload_ty.clone()],
            ret: Ty::Sum(decl.name.clone()),
            ffi_symbol: None,
            is_action: false,
            is_generic: false,
            is_constructor: true,
            associative_neutral: None,
            type_params: vec![],
            substitutions: None,
            param_names: vec![],
        });
    }

    // ── Passo 2: sintetiza TypedFunctions ──
    // Construtores enum_pred não usam interfaces — registry vazio.
    let empty_iface_reg = kata_core::interface_registry::InterfaceRegistry::new();
    let empty_refines_reg = kata_core::RefinesRegistry::new();
    let deferred = super::expr::DeferredLambdaTable::default();
    let ctx = InferCtx {
        table: &*dispatch_table,
        enum_registry,
        struct_registry,
        refined_decls: &[],
        interface_registry: &empty_iface_reg,
        refines_registry: &empty_refines_reg,
        ret_ty: None,
        in_loop: false,
        deferred_lambdas: &deferred,
    };

    let mut functions = Vec::new();

    for decl in enum_pred_decls {
        let enum_ty = Ty::Sum(decl.name.clone());

        // ── 2a. Sintetiza funções predicado ──
        for variant in &decl.variants {
            if let Some(pred_expr) = &variant.predicate {
                let pred_name = format!("__pred_enum_{}_{}", decl.name, variant.name);

                // Substitui Hole por Ident("x") no predicado
                let substituted = substitute_hole(pred_expr, "x");
                let desugared = desugar::desugar(&substituted);

                // Cria TypeEnv com x: payload_ty
                let mut pred_env = type_env.clone();
                pred_env.define("x", decl.payload_ty.clone(), "__local__");

                // Infere o body do predicado
                let typed_body = infer_expr(
                    &desugared.node,
                    &desugared.span,
                    &mut pred_env,
                    &ctx,
                    true, // tail_pos
                )?;

                let pattern = Spanned::new(
                    TypedPattern::Ident {
                        name: "x".into(),
                        ty: decl.payload_ty.clone(),
                    },
                    Span::synthetic(),
                );

                functions.push(TypedFunction {
                    name: pred_name,
                    param_types: vec![decl.payload_ty.clone()],
                    ret_ty: Ty::boolean(),
                    clauses: vec![TypedLambdaClause {
                        patterns: vec![pattern],
                        body: Spanned::new(typed_body, desugared.span),
                        guards: Vec::new(),
                        with_bindings: Vec::new(),
                    }],
                    log: None,
                    cache_spec: None,
            timer_spec: None,
                });
            }
        }

        // ── 2b. Sintetiza construtor despachador com guard chain ──
        // IMC :: Float => IMC
        // lambda x:
        //     __pred_enum_IMC_Magreza(x): Magreza(x)
        //     __pred_enum_IMC_Normal(x): Normal(x)
        //     otherwise: Obesidade(x)
        let mut guards = Vec::new();

        for variant in &decl.variants {
            if variant.predicate.is_some() {
                // Guard: __pred_enum_EnumName_Variant(x) → VariantConstruct(variant, x)
                let pred_call = build_pred_call(
                    &format!("__pred_enum_{}_{}", decl.name, variant.name),
                    "x",
                    &decl.payload_ty,
                );
                let body = build_variant_construct(
                    &decl.name,
                    &variant.name,
                    "x",
                    &decl.payload_ty,
                    variant.tag,
                );
                guards.push(TypedGuardClause {
                    condition: Some(pred_call),
                    body,
                });
            }
        }

        // Último guard: otherwise → variante default (sem predicado)
        if let Some(default_variant) = decl.variants.iter().find(|v| v.predicate.is_none()) {
            let body = build_variant_construct(
                &decl.name,
                &default_variant.name,
                "x",
                &decl.payload_ty,
                default_variant.tag,
            );
            guards.push(TypedGuardClause {
                condition: None,
                body,
            });
        }

        let pattern = Spanned::new(
            TypedPattern::Ident {
                name: "x".into(),
                ty: decl.payload_ty.clone(),
            },
            Span::synthetic(),
        );

        // O body da clause quando há guards é o primeiro guard body;
        // o codegen percorre guards em sequência.
        let body = guards.first().map(|g| g.body.clone()).unwrap_or_else(|| {
            // Fallback defensivo: se não há guards, retorna a variante default
            build_variant_construct(
                &decl.name,
                &decl.variants.last().expect("enum deve ter variantes").name,
                "x",
                &decl.payload_ty,
                decl.variants.len() - 1,
            )
        });

        functions.push(TypedFunction {
            name: decl.name.clone(),
            param_types: vec![decl.payload_ty.clone()],
            ret_ty: enum_ty,
            clauses: vec![TypedLambdaClause {
                patterns: vec![pattern],
                body,
                guards,
                with_bindings: Vec::new(),
            }],
            log: None,
            cache_spec: None,
            timer_spec: None,
        });
    }

    Ok(functions)
}

/// Constrói `VariantConstruct { enum_name, variant, payload: Ident(var_name), tag }`.
fn build_variant_construct(
    enum_name: &str,
    variant: &str,
    var_name: &str,
    payload_ty: &Ty,
    tag: usize,
) -> Spanned<TypedExpr> {
    let payload = TypedExpr {
        span: Span::synthetic(),
        ty: payload_ty.clone(),
        tail_pos: false,
        escape: EscapeTarget::Local,
        kind: TypedExprKind::Ident {
            name: var_name.into(),
        },
    };
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: Ty::Sum(enum_name.into()),
            tail_pos: true,
            escape: EscapeTarget::Caller,
            kind: TypedExprKind::VariantConstruct {
                enum_name: enum_name.into(),
                variant: variant.into(),
                payload: Box::new(Spanned::new(payload, Span::synthetic())),
                tag,
                module_path: None,
            },
        },
        Span::synthetic(),
    )
}
