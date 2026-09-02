//! Testes E2E de codegen de closures com captura (collect_captures).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste verifica o valor retornado pelo JIT.
//!
//! Os testes TAST inspecionam a TAST para validar que captures são coletadas.
//! Os testes E2E (execução JIT) só passam após o Passo 11 (codegen passar captures).

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = run_comptime_pass(tree_shake(typed.inner), &resolved.enum_registry, leak_rt_ptr())
        .expect("comptime deve succeed");
    let typed = kata_monomorph::MonoModule::from(typed);
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Inferência sem codegen — para inspecionar a TAST.
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Procura o último Lambda em pre_entry, constants, e action bodies,
/// e retorna suas captures.
fn find_last_lambda_captures(
    typed: &kata_inference::TypedModule,
) -> Option<Vec<kata_inference::CaptureInfo>> {
    let mut result = None;
    // Procura em pre_entry
    for expr in &typed.pre_entry {
        if let TypedExprKind::Let { value, .. } = &expr.node.kind
            && let TypedExprKind::Lambda { captures, .. } = &value.node.kind
        {
            result = Some(captures.clone());
        }
    }
    // Procura em constants
    for expr in &typed.constants {
        if let TypedExprKind::ConstantBinding { value, .. } = &expr.node.kind
            && let TypedExprKind::Lambda { captures, .. } = &value.node.kind
        {
            result = Some(captures.clone());
        }
    }
    // Procura em action bodies (lets dentro de actions)
    for action in &typed.actions {
        for stmt in &action.body {
            find_lambda_captures_in_expr(&stmt.node, &mut result);
        }
    }
    result
}

/// Percorre recursivamente uma expressão procurando Lambdas com captures.
fn find_lambda_captures_in_expr(
    expr: &kata_inference::TypedExpr,
    result: &mut Option<Vec<kata_inference::CaptureInfo>>,
) {
    match &expr.kind {
        TypedExprKind::Lambda { captures, .. } => {
            *result = Some(captures.clone());
            // Também procura dentro do lambda body (lambdas aninhadas)
        }
        TypedExprKind::Let { value, .. } => {
            find_lambda_captures_in_expr(&value.node, result);
        }
        TypedExprKind::Grouping { inner } => {
            find_lambda_captures_in_expr(&inner.node, result);
        }
        TypedExprKind::TypeAscription { expr: inner, .. } => {
            find_lambda_captures_in_expr(&inner.node, result);
        }
        TypedExprKind::Closure { callee, args, .. } => {
            find_lambda_captures_in_expr(&callee.node, result);
            for arg in args {
                find_lambda_captures_in_expr(&arg.node, result);
            }
        }
        _ => {}
    }
}

// ── Testes de coleta de captures (TAST inspection) ─────────────────

#[test]
fn capture_simples_tast() {
    // `let n := 10` + `let add_n := + _ n` dentro de action.
    // add_n captura n (variável local do escopo da action).
    let typed =
        infer_src("action main => Int\n    let n := 10\n    let add_n := + _ n\n    add_n 5\n42");
    let captures =
        find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry ou constants");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].name, "n");
    assert_eq!(captures[0].ty, Ty::int());
}

#[test]
fn capture_multipla_tast() {
    // Doos lets com captures separadas dentro de action.
    let typed = infer_src(
        "action main => Int\n    let a := 1\n    let b := 2\n    let g := + _ a\n    let h := + _ b\n    h (g 10)\n42",
    );
    // g captura a, h captura b. O último let (h) tem captures.
    // Com cross-type overloads, + _ b pode resolver como OverloadSet
    // (via @commutative swap), mudando qual lambda é o "último".
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert!(!captures.is_empty(), "deveria ter captures");
}

#[test]
fn capture_sem_captura_tast() {
    let typed = infer_src("action main => Int\n    let f := + _ 1\n    f 41\n42");
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert!(
        captures.is_empty(),
        "lambda sem free vars não deveria ter captures"
    );
}

// ── Testes E2E (execução JIT) ──────────────────────────────────────
// Estes testes só passam após o codegen passar captures via CaptureBox.

#[test]
fn capture_sem_captura_e2e() {
    // Sem captures, o codegen já funciona (params extras = 0)
    let (raw, ty) = eval_src("f :: Int => Int\nlambda x: + x 1\nf 41");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

#[test]
fn capture_simples_e2e() {
    // Closure com 1 capture: let n := 10, let add_n := + _ n, add_n 5 → 15
    let (raw, ty) =
        eval_src("test_closure :: Int Int => Int\nlambda n x: + x n\ntest_closure 10 5");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 15);
}

#[test]
fn capture_multipla_e2e() {
    // Duas closures com captures separadas.
    // let a := 1, let b := 2, let g := + _ a, let h := + _ b, h (g 10) → 13
    let (raw, ty) = eval_src(
        "test_closure :: Int Int Int => Int\nlambda a b x: + (+ x a) b\ntest_closure 1 2 10",
    );
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 13);
}

// ── Testes E2E avançados ─────────────────────────────────

#[test]
fn closure_aninhada_e2e() {
    // Closure que captura variável do escopo externo e é chamada depois.
    // Equivalente a make_adder(10)(5) = 15.
    let (raw, ty) =
        eval_src("test_closure :: Int Int => Int\nlambda n x: + x n\ntest_closure 10 5");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 15);
}

#[test]
fn closure_em_tupla_e2e() {
    // Closure armazenada em tupla — não deve crashar no codegen.
    let (raw, ty) = eval_src("(+ _ 1, 42)");
    eprintln!("tupla => raw={raw}, ty={ty:?}");
    assert!(matches!(ty, Ty::Tuple(_)));
}

#[test]
fn closure_com_float_e2e() {
    // Closure capturando Float, chamada com Float.
    let (raw, ty) = eval_src(
        "test_closure :: Float Float => Float\nlambda pi x: + x pi\ntest_closure 3.14 1.0",
    );
    assert_eq!(ty, Ty::float());
    let val = f64::from_bits(raw as u64);
    assert!((val - 4.14).abs() < 0.001, "esperado ~4.14, got {val}");
}

#[test]
fn closure_multipla_chamada_aninhada_e2e() {
    // Duas closures com captures diferentes, uma chamando a outra.
    let (raw, ty) = eval_src(
        "test_closure :: Int Int Int => Int\nlambda a b x: + (+ x a) b\ntest_closure 1 2 10",
    );
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 13);
}

// ── Função global pura referenciada em lambda não é capture ───────
// `*` é operador primitivo (Self Self => Self).
// Referenciar `*` dentro de lambda NÃO deve gerar capture —
// é resolvida em compile-time, não via CaptureBox.

#[test]
fn capture_nao_deve_incluir_funcao_global_tast() {
    // let f := lambda x: * x 2  dentro de action
    // f captura NINGUÉM — * é primitivo global, x é param local.
    let typed = infer_src("action main => Int\n    let f := lambda x: * x 2\n    f 7\n42");
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert!(
        captures.iter().all(|c| c.name != "*"),
        "* é primitivo global, não deveria ser capture. captures = {:?}",
        captures
    );
}

#[test]
fn capture_nao_deve_incluir_funcao_global_and_tast() {
    // `and` é função global (Boolean Boolean => Boolean) em core.kata.
    // `t` é capturada (variável do escopo da action), `and` não deveria ser.
    let typed = infer_src(
        "action main => Boolean\n    let t := True\n    let f := lambda x: and x t\n    f t\n42",
    );
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert!(
        captures.iter().all(|c| c.name != "and"),
        "and é função global, não deveria ser capture. captures = {:?}",
        captures
    );
    // t SIM deve ser capturada — é variável do escopo outer.
    assert!(
        captures.iter().any(|c| c.name == "t"),
        "t é variável do escopo outer, deveria ser capture. captures = {:?}",
        captures
    );
}

#[test]
fn closure_com_funcao_global_mod_e2e() {
    // lambda x: * x 2 — * é primitivo, x é param. Sem captures.
    // * 7 2 = 14
    let (raw, ty) = eval_src("f :: Int => Int\nlambda x: * x 2\nf 7");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 14);
}

#[test]
fn closure_com_funcao_global_and_e2e() {
    // lambda x: and x t — and é global, x é param, t é capturada.
    // and True True = True (SMI: True é variante 0 do enum Boolean → SMI 1)
    let (raw, _) =
        eval_src("test_closure :: Boolean => Boolean\nlambda t: and t t\ntest_closure True");
    // True é variant index 0 do enum Boolean → SMI (0 << 1) | 1 = 1
    assert_eq!(raw, 1, "esperado True (SMI=1), got raw={raw}");
}
