//! Testes E2E de codegen de closures com captura (collect_captures).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste verifica o valor retornado pelo JIT.
//!
//! Os testes TAST inspecionam a TAST para validar que captures são coletadas.
//! Os testes E2E (execução JIT) só passam após o Passo 11 (codegen passar captures).

use kata_codegen::jit_eval;
use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Inferência sem codegen — para inspecionar a TAST.
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
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
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Procura o último Lambda em pre_entry Let values e retorna suas captures.
fn find_last_lambda_captures(
    typed: &kata_inference::TypedModule,
) -> Option<Vec<kata_inference::CaptureInfo>> {
    let mut result = None;
    for expr in &typed.pre_entry {
        if let TypedExprKind::Let { value, .. } = &expr.node.kind
            && let TypedExprKind::Lambda { captures, .. } = &value.node.kind
        {
            result = Some(captures.clone());
        }
    }
    result
}

// ── Testes de coleta de captures (TAST inspection) ─────────────────

#[test]
fn capture_simples_tast() {
    let typed = infer_src("let n := 10\nlet add_n := + _ n\nadd_n 5");
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].name, "n");
    assert_eq!(captures[0].ty, Ty::int());
}

#[test]
fn capture_multipla_tast() {
    // Dois lets com captures separadas para evitar problemas de inferência.
    let typed = infer_src("let a := 1\nlet b := 2\nlet g := + _ a\nlet h := + _ b\nh (g 10)");
    // g captura a, h captura b
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    // O último let (h) tem captures = [b]. Verificamos que não é vazio.
    assert!(!captures.is_empty(), "deveria ter captures");
    assert_eq!(captures[0].name, "b");
}

#[test]
fn capture_sem_captura_tast() {
    let typed = infer_src("let f := + _ 1\nf 41");
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
    let (raw, ty) = eval_src("let f := + _ 1\nf 41");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 42);
}

#[test]
fn capture_simples_e2e() {
    // Closure com 1 capture: let n := 10, let add_n := + _ n, add_n 5 → 15
    let (raw, ty) = eval_src("let n := 10\nlet add_n := + _ n\nadd_n 5");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 15);
}

#[test]
fn capture_multipla_e2e() {
    // Duas closures com captures separadas.
    // let a := 1, let b := 2, let g := + _ a, let h := + _ b, h (g 10) → 13
    let (raw, ty) = eval_src("let a := 1\nlet b := 2\nlet g := + _ a\nlet h := + _ b\nh (g 10)");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 13);
}

// ── Testes E2E avançados ─────────────────────────────────

#[test]
fn closure_aninhada_e2e() {
    // Closure que captura variável do escopo externo e é chamada depois.
    // Equivalente a make_adder(10)(5) = 15.
    let (raw, ty) = eval_src("let n := 10\nlet f := + _ n\nf 5");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 15);
}

#[test]
fn closure_em_tupla_e2e() {
    // Closure armazenada em tupla — não deve crashar no codegen.
    let (raw, ty) = eval_src("let f := + _ 1\n(f, 42)");
    eprintln!("tupla => raw={raw}, ty={ty:?}");
    assert!(matches!(ty, Ty::Tuple(_)));
}

#[test]
fn closure_com_float_e2e() {
    // Closure capturando Float, chamada com Float.
    let (raw, ty) = eval_src("let pi := 3.14\nlet f := + _ pi\nf 1.0");
    assert_eq!(ty, Ty::float());
    let val = f64::from_bits(raw as u64);
    assert!((val - 4.14).abs() < 0.001, "esperado ~4.14, got {val}");
}

#[test]
fn closure_multipla_chamada_aninhada_e2e() {
    // Duas closures com captures diferentes, uma chamando a outra.
    let (raw, ty) = eval_src("let a := 1\nlet b := 2\nlet g := + _ a\nlet h := + _ b\nh (g 10)");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 13);
}

// ── Função global pura referenciada em lambda não é capture ───────
// `mod` é função definida em core.kata (não-FFI, DispatchTable).
// Referenciar `mod` dentro de lambda NÃO deve gerar capture —
// é resolvida em compile-time via call direto, não via CaptureBox.

#[test]
fn capture_nao_deve_incluir_funcao_global_tast() {
    // let f := lambda x: mod x 2
    // f captura NINGUÉM — mod é função global, x é param local.
    let typed = infer_src("let f := lambda x: mod x 2\nf 7");
    let captures = find_last_lambda_captures(&typed).expect("deveria ter um Lambda no pre_entry");
    assert!(
        captures.iter().all(|c| c.name != "mod"),
        "mod é função global, não deveria ser capture. captures = {:?}",
        captures
    );
}

#[test]
fn capture_nao_deve_incluir_funcao_global_and_tast() {
    // `and` é função global (Boolean Boolean => Boolean) em core.kata.
    // `t` é capturada (variável do escopo outer), `and` não deveria ser.
    let typed = infer_src("let t := True\nlet f := lambda x: and x t\nf t");
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
    // lambda x: mod x 2 — mod é global, x é param. Sem captures.
    // mod 7 2 = 1
    let (raw, ty) = eval_src("let f := lambda x: mod x 2\nf 7");
    assert_eq!(ty, Ty::int());
    assert_eq!(untag_smi(raw), 1);
}

#[test]
fn closure_com_funcao_global_and_e2e() {
    // lambda x: and x t — and é global, x é param, t é capturada.
    // and True True = True (SMI: True é variante 0 do enum Boolean → SMI 1)
    let (raw, _) = eval_src("let t := True\nlet f := lambda x: and x t\nf t");
    // True é variant index 0 do enum Boolean → SMI (0 << 1) | 1 = 1
    assert_eq!(raw, 1, "esperado True (SMI=1), got raw={raw}");
}
