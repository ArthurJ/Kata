//! Fio 6 — Parser: refined declarations e enum predicado.

use super::helpers::{first_item, parse_src};
use kata_ast::{Expr, Item, TypeExpr};

// ── Refined declarations ───────────────────────────────────────────

#[test]
fn refined_decl_simple() {
    let m = parse_src("data (Int, > _ 0) as PositiveInt");
    let item = first_item(&m);
    match item {
        Item::DataDecl {
            name,
            fields,
            refined,
            ..
        } => {
            assert_eq!(name, "PositiveInt");
            assert!(fields.is_empty());
            let refined = refined.as_ref().expect("refined must be Some");
            assert_eq!(refined.base_ty.node, TypeExpr::Named("Int".into()));
            assert_eq!(refined.predicates.len(), 1);
            // Predicado: > _ 0 → Apply { Ident(">"), [Hole, IntLit("0")] }
            match &refined.predicates[0].node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: ">".into() });
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[0].node, Expr::Hole);
                    assert_eq!(args[1].node, Expr::IntLit { text: "0".into() });
                }
                other => panic!("expected Apply, got {other:?}"),
            }
        }
        other => panic!("expected DataDecl with refined, got {other:?}"),
    }
}

#[test]
fn refined_decl_multiple_predicates() {
    let m = parse_src("data (Int, > _ 0, <= _ 100) as Percentage");
    let item = first_item(&m);
    match item {
        Item::DataDecl { name, refined, .. } => {
            assert_eq!(name, "Percentage");
            let refined = refined.as_ref().expect("refined must be Some");
            assert_eq!(refined.base_ty.node, TypeExpr::Named("Int".into()));
            assert_eq!(refined.predicates.len(), 2);
            // Primeiro predicado: > _ 0
            match &refined.predicates[0].node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: ">".into() });
                    assert_eq!(args[0].node, Expr::Hole);
                }
                other => panic!("expected Apply, got {other:?}"),
            }
            // Segundo predicado: <= _ 100
            match &refined.predicates[1].node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: "<=".into() });
                    assert_eq!(args[0].node, Expr::Hole);
                    assert_eq!(args[1].node, Expr::IntLit { text: "100".into() });
                }
                other => panic!("expected Apply, got {other:?}"),
            }
        }
        other => panic!("expected DataDecl with refined, got {other:?}"),
    }
}

#[test]
fn refined_decl_float_base() {
    let m = parse_src("data (Float, >= _ 0.0) as NonNegFloat");
    let item = first_item(&m);
    match item {
        Item::DataDecl { name, refined, .. } => {
            assert_eq!(name, "NonNegFloat");
            let refined = refined.as_ref().expect("refined must be Some");
            assert_eq!(refined.base_ty.node, TypeExpr::Named("Float".into()));
            assert_eq!(refined.predicates.len(), 1);
            match &refined.predicates[0].node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: ">=".into() });
                    assert_eq!(args[0].node, Expr::Hole);
                    assert_eq!(args[1].node, Expr::FloatLit { text: "0.0".into() });
                }
                other => panic!("expected Apply, got {other:?}"),
            }
        }
        other => panic!("expected DataDecl with refined, got {other:?}"),
    }
}

// ── Enum predicado ─────────────────────────────────────────────────

#[test]
fn enum_predicado_simple() {
    let m = parse_src("enum IMC\n    Magreza(< _ 18.5)\n    Normal(<= _ 25.0)\n    Obesidade");
    let item = first_item(&m);
    match item {
        Item::EnumDecl { name, variants, .. } => {
            assert_eq!(name, "IMC");
            assert_eq!(variants.len(), 3);

            // Magreza: predicado < _ 18.5
            assert_eq!(variants[0].name, "Magreza");
            assert!(variants[0].payload.is_none());
            let pred = variants[0]
                .predicate
                .as_ref()
                .expect("predicate must be Some");
            match &pred.node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: "<".into() });
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[0].node, Expr::Hole);
                    assert_eq!(
                        args[1].node,
                        Expr::FloatLit {
                            text: "18.5".into()
                        }
                    );
                }
                other => panic!("expected Apply, got {other:?}"),
            }

            // Normal: predicado <= _ 25.0
            assert_eq!(variants[1].name, "Normal");
            let pred = variants[1]
                .predicate
                .as_ref()
                .expect("predicate must be Some");
            match &pred.node {
                Expr::Apply { callee, args } => {
                    assert_eq!(callee.node, Expr::Ident { name: "<=".into() });
                    assert_eq!(
                        args[1].node,
                        Expr::FloatLit {
                            text: "25.0".into()
                        }
                    );
                }
                other => panic!("expected Apply, got {other:?}"),
            }

            // Obesidade: sem predicado (default)
            assert_eq!(variants[2].name, "Obesidade");
            assert!(variants[2].predicate.is_none());
        }
        other => panic!("expected EnumDecl, got {other:?}"),
    }
}

#[test]
fn enum_predicado_and_payload_mixed() {
    // Enum com variante com payload e variante com predicado
    let m = parse_src("enum Estado\n    Ativo(Int)\n    Inativo(<= _ 0)\n    Encerrado");
    let item = first_item(&m);
    match item {
        Item::EnumDecl { name, variants, .. } => {
            assert_eq!(name, "Estado");
            assert_eq!(variants.len(), 3);

            // Ativo: payload Int, sem predicado
            assert_eq!(variants[0].name, "Ativo");
            assert!(variants[0].payload.is_some());
            assert!(variants[0].predicate.is_none());

            // Inativo: predicado, sem payload
            assert_eq!(variants[1].name, "Inativo");
            assert!(variants[1].payload.is_none());
            assert!(variants[1].predicate.is_some());

            // Encerrado: sem nada
            assert_eq!(variants[2].name, "Encerrado");
            assert!(variants[2].payload.is_none());
            assert!(variants[2].predicate.is_none());
        }
        other => panic!("expected EnumDecl, got {other:?}"),
    }
}

// ── Compatibilidade: struct normal e enum sem predicado ───────────

#[test]
fn struct_normal_nao_refined() {
    let m = parse_src("data Pessoa (nome::Text idade::Int)");
    let item = first_item(&m);
    match item {
        Item::DataDecl {
            name,
            fields,
            refined,
            ..
        } => {
            assert_eq!(name, "Pessoa");
            assert_eq!(fields.len(), 2);
            assert!(refined.is_none());
        }
        other => panic!("expected DataDecl, got {other:?}"),
    }
}

#[test]
fn enum_sem_predicado_continua_funcionando() {
    let m = parse_src("enum Optional\n    Some(Int)\n    None");
    let item = first_item(&m);
    match item {
        Item::EnumDecl { name, variants, .. } => {
            assert_eq!(name, "Optional");
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].name, "Some");
            assert!(variants[0].payload.is_some());
            assert!(variants[0].predicate.is_none());
            assert_eq!(variants[1].name, "None");
            assert!(variants[1].payload.is_none());
            assert!(variants[1].predicate.is_none());
        }
        other => panic!("expected EnumDecl, got {other:?}"),
    }
}
