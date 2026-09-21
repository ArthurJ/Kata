//! Testes do mecanismo `#!` — Fase 1 do PRD-pragma-mechanism.
//!
//! T1: `#!allow` → DiagnosticControl { level: Allow }
//! T2: `#!warn` → DiagnosticControl { level: Warn }
//! T3: `#!deny` → DiagnosticControl { level: Deny }
//! T4: `#!bench-config` (prefixo) → UnknownPragma
//! T5: `#!mylint-max_line` inline → UnknownPragma (lexer tokeniza)
//! T6: `#!benchmark` (sem prefixo) → erro de parser

use kata_ast::{DiagnosticLevel, Item, Pragma};
use kata_lexer::lex;
use kata_parser::parse;

fn parse_pragma(src: &str) -> Vec<Pragma> {
    let tokens = lex(src).expect("lex failed");
    let module = parse(tokens).expect("parse failed");
    // Pragmas podem estar em module.pragmas (órfãos) ou anexados
    // a ModuleEntry (escopo posicional).
    let mut all = module.pragmas.clone();
    for entry in &module.items {
        all.extend(entry.pragmas.clone());
    }
    all
}

fn parse_pragma_err(src: &str) -> String {
    let tokens = lex(src).expect("lex failed");
    match parse(tokens) {
        Ok(_) => panic!("expected parse error, got Ok"),
        Err(e) => e.to_string(),
    }
}

/// Programa mínimo válido para acompanhar pragmas.
/// `echo!(\"hi\")` é a última expressão top-level (entry point).
const MINIMAL_PROG: &str = "echo!(\"hi\")\n";

#[test]
fn t1_pragma_allow_produces_diagnostic_control() {
    let pragmas = parse_pragma(&format!(
        "#!allow type.incomplete_interface\n{MINIMAL_PROG}"
    ));
    assert_eq!(pragmas.len(), 1);
    match &pragmas[0] {
        Pragma::DiagnosticControl(dc) => {
            assert_eq!(dc.level, DiagnosticLevel::Allow);
            assert_eq!(dc.code, "type.incomplete_interface");
        }
        other => panic!("expected DiagnosticControl, got {other:?}"),
    }
}

#[test]
fn t2_pragma_warn_produces_diagnostic_control() {
    let pragmas = parse_pragma(&format!("#!warn type.redundant_clause\n{MINIMAL_PROG}"));
    assert_eq!(pragmas.len(), 1);
    match &pragmas[0] {
        Pragma::DiagnosticControl(dc) => {
            assert_eq!(dc.level, DiagnosticLevel::Warn);
            assert_eq!(dc.code, "type.redundant_clause");
        }
        other => panic!("expected DiagnosticControl, got {other:?}"),
    }
}

#[test]
fn t3_pragma_deny_produces_diagnostic_control() {
    let pragmas = parse_pragma(&format!("#!deny type.incomplete_interface\n{MINIMAL_PROG}"));
    assert_eq!(pragmas.len(), 1);
    match &pragmas[0] {
        Pragma::DiagnosticControl(dc) => {
            assert_eq!(dc.level, DiagnosticLevel::Deny);
            assert_eq!(dc.code, "type.incomplete_interface");
        }
        other => panic!("expected DiagnosticControl, got {other:?}"),
    }
}

#[test]
fn t4_pragma_external_with_prefix_preserved() {
    let pragmas = parse_pragma(&format!("#!bench-config iterations: 1000\n{MINIMAL_PROG}"));
    assert_eq!(pragmas.len(), 1);
    match &pragmas[0] {
        Pragma::UnknownPragma(up) => {
            assert_eq!(up.token, "bench-config");
            assert_eq!(up.prefix, "bench");
            assert_eq!(up.raw, "iterations: 1000");
        }
        other => panic!("expected UnknownPragma, got {other:?}"),
    }
}

#[test]
fn t5_pragma_external_inline_lexer_tokenizes() {
    // Inline pragma no fim de uma linha — o lexer produz o token.
    // O parser ainda não anexa inline na Fase 1.
    let src = "echo!(\"hi\")  #!mylint-max_line 80\n";
    let tokens = lex(src).expect("lex failed");
    let has_pragma = tokens
        .iter()
        .any(|t| matches!(t.token, kata_ast::Token::Pragma { .. }));
    assert!(
        has_pragma,
        "lexer should produce a Pragma token for inline #!mylint-max_line"
    );
}

#[test]
fn t6_pragma_without_prefix_rejected() {
    let err = parse_pragma_err(&format!("#!benchmark iterations: 1000\n{MINIMAL_PROG}"));
    assert!(
        err.contains("benchmark") || err.contains("pragma"),
        "error should mention `benchmark` or `pragma`, got: {err}"
    );
}

#[test]
fn t7_multiple_pragmas_collected() {
    let pragmas = parse_pragma(&format!(
        "#!allow type.incomplete_interface\n#!warn type.redundant_clause\n{MINIMAL_PROG}"
    ));
    assert_eq!(pragmas.len(), 2);
    assert!(matches!(
        &pragmas[0],
        Pragma::DiagnosticControl(dc) if dc.level == DiagnosticLevel::Allow
    ));
    assert!(matches!(
        &pragmas[1],
        Pragma::DiagnosticControl(dc) if dc.level == DiagnosticLevel::Warn
    ));
}

#[test]
fn t8_pragma_test_marker_produces_test_spec() {
    // #!test antes de uma action — anexa à ActionDecl.
    let src = "#!test(\"soma correta\")\naction foo => Int\n    42\n";
    let tokens = lex(src).expect("lex failed");
    let module = parse(tokens).expect("parse failed");
    // #!test não vai para Module.pragmas — vai para ActionDecl.pragmas.
    assert!(
        module.pragmas.is_empty(),
        "#!test should be attached to ActionDecl, not Module.pragmas"
    );
    let action = module
        .items
        .iter()
        .find_map(|i| match &i.item.node {
            Item::ActionDecl { pragmas, .. } => Some(pragmas),
            _ => None,
        })
        .expect("should have an ActionDecl");
    assert_eq!(action.len(), 1);
    match &action[0] {
        Pragma::TestSpec(ts) => {
            assert_eq!(ts.desc, "soma correta");
            assert!(ts.args.is_empty());
            assert!(ts.timeout.is_none());
        }
        other => panic!("expected TestSpec, got {other:?}"),
    }
}

#[test]
fn t9_pragma_test_with_timeout() {
    // #!test{desc: "...", timeout: N} antes de uma action.
    let src = "#!test{desc: \"com timeout\", timeout: 5000}\naction foo => Int\n    42\n";
    let tokens = lex(src).expect("lex failed");
    let module = parse(tokens).expect("parse failed");
    assert!(module.pragmas.is_empty());
    let action = module
        .items
        .iter()
        .find_map(|i| match &i.item.node {
            Item::ActionDecl { pragmas, .. } => Some(pragmas),
            _ => None,
        })
        .expect("should have an ActionDecl");
    assert_eq!(action.len(), 1);
    match &action[0] {
        Pragma::TestSpec(ts) => {
            assert_eq!(ts.desc, "com timeout");
            assert_eq!(ts.timeout, Some(5000));
        }
        other => panic!("expected TestSpec, got {other:?}"),
    }
}

#[test]
fn t10_pragma_does_not_break_decl_after() {
    // Pragma antes de uma declaração sig deve ser coletado sem
    // impedir o parse da sig. Escopo posicional: pragma anexado
    // ao ModuleEntry da sig.
    let src = "#!allow type.incomplete_interface\n+ :: Int Int => Int\n";
    let tokens = lex(src).expect("lex failed");
    let module = parse(tokens).expect("parse failed");
    // Pragma pode estar em module.pragmas ou anexado ao ModuleEntry.
    let pragma_count = module.pragmas.len()
        + module.items.iter().map(|e| e.pragmas.len()).sum::<usize>();
    assert_eq!(pragma_count, 1);
    let has_sig = module
        .items
        .iter()
        .any(|item| matches!(item.item.node, Item::Sig { .. }));
    assert!(has_sig, "module should contain the sig decl");
}

#[test]
fn t11_regular_comment_still_works() {
    let src = "# This is a comment\necho!(\"hi\")\n";
    let tokens = lex(src).expect("lex failed");
    let has_pragma = tokens
        .iter()
        .any(|t| matches!(t.token, kata_ast::Token::Pragma { .. }));
    assert!(
        !has_pragma,
        "regular comment should not produce Pragma token"
    );
}

#[test]
fn t12_multiline_comment_still_works() {
    let src = "#{ this is a\nmultiline comment }#\necho!(\"hi\")\n";
    let tokens = lex(src).expect("lex failed");
    let has_pragma = tokens
        .iter()
        .any(|t| matches!(t.token, kata_ast::Token::Pragma { .. }));
    assert!(
        !has_pragma,
        "multiline comment should not produce Pragma token"
    );
}
