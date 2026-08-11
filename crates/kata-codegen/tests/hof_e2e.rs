//! Testes E2E de codegen de map/filter/fold (higher-order over collections).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Valida DoDs 57-59: map, filter, fold sobre List/Array/Range.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
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

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);

    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
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

/// Lê um ponteiro de Cons cell e extrai o primeiro elemento (head).
/// Cons cell layout: [head: i64 | tail_ptr: i64] (offset 0 = head, offset 8 = tail)
/// Retorno é o head como i64.
#[allow(dead_code)]
fn cons_head(raw: i64) -> i64 {
    if raw == 0 {
        return 0; // Nil
    }
    unsafe {
        let ptr = raw as *const i64;
        // Layout: ptr+0 = head, ptr+8 = tail_ptr
        *ptr
    }
}

// ═══════════════════════════════════════════════════════════════
// DoD 57: map (+ 10 _) [1 2 3] → [11 12 13]
// ═══════════════════════════════════════════════════════════════

/// `map (lambda x: + x 10) [1 2 3]` → List(Int) com 11, 12, 13.
/// O callback soma 10 a cada elemento. map retorna List(Int).
#[test]
fn map_soma_10_em_list_retorna_list_int() {
    let src = "map (lambda x: + x 10) [1 2 3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "map (+x 10) [1 2 3] deve retornar List(Int)"
    );
    // List é ponteiro para Cons cell (não-zero, não-SMI).
    assert_ne!(raw, 0, "lista não-vazia não deve ser ponteiro nulo");
    // Verifica o primeiro elemento: head da Cons = 11 (SMI-tagged = 23).
    assert_eq!(untag_smi(cons_head(raw)), 11, "primeiro elemento = 11");
}

/// `map (+ 10 _) [1 2 3]` com Hole — desugar transforma em lambda.
#[test]
fn map_com_hole_soma_10() {
    let src = "map (+ 10 _) [1 2 3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "map (+ 10 _) [1 2 3] deve retornar List(Int)"
    );
    assert_ne!(raw, 0, "lista não-vazia");
    assert_eq!(untag_smi(cons_head(raw)), 11, "primeiro elemento = 11");
}

// ═══════════════════════════════════════════════════════════════
// DoD 58: filter (> _ 5) [1 8 3 9] → [8 9]
// ═══════════════════════════════════════════════════════════════

/// `filter (lambda x: > x 5) [1 8 3 9]` → List(Int) com 8, 9.
/// O predicado retorna Boolean. filter retorna List(Int).
#[test]
fn filter_maior_que_5_em_list() {
    let src = "filter (lambda x: > x 5) [1 8 3 9]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "filter (>x 5) [1 8 3 9] deve retornar List(Int)"
    );
    assert_ne!(raw, 0, "lista não-vazia (8, 9)");
    // Primeiro elemento = 8 (SMI-tagged = 17).
    assert_eq!(untag_smi(cons_head(raw)), 8, "primeiro elemento = 8");
}

/// `filter (> _ 5) [1 8 3 9]` com Hole.
#[test]
fn filter_com_hole_maior_que_5() {
    let src = "filter (> _ 5) [1 8 3 9]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "filter (> _ 5) [1 8 3 9] deve retornar List(Int)"
    );
    assert_ne!(raw, 0, "lista não-vazia");
    assert_eq!(untag_smi(cons_head(raw)), 8, "primeiro elemento = 8");
}

// ═══════════════════════════════════════════════════════════════
// DoD 59: fold + 0 [1 2 3] → 6
// ═══════════════════════════════════════════════════════════════

/// `fold + 0 [1 2 3]` → 6 (soma acumulada 0+1+2+3).
/// O callback é `+` (Ident), initial=0, collection=[1 2 3].
/// fold retorna acc_ty = Int.
#[test]
fn fold_soma_list_retorna_6() {
    let src = "fold + 0 [1 2 3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "fold + 0 [1 2 3] deve retornar Int");
    assert_eq!(untag_smi(raw), 6, "fold + 0 [1 2 3] = 6");
}

/// `fold (lambda acc x: + acc x) 0 [1 2 3]` com lambda explícito.
#[test]
fn fold_com_lambda_soma_retorna_6() {
    let src = "fold (lambda acc x: + acc x) 0 [1 2 3]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "fold lambda deve retornar Int");
    assert_eq!(untag_smi(raw), 6, "fold + 0 [1 2 3] = 6");
}

// ═══════════════════════════════════════════════════════════════
// Casos extras: map sobre Array, fold com multiplicação
// ═══════════════════════════════════════════════════════════════

/// `map (lambda x: + x 1) {1 2 3}` → map sobre Array retorna List(Int).
/// O codegen converte List→Array no final se input era Array.
#[test]
fn map_sobre_array_retorna_collection() {
    let src = "map (lambda x: + x 1) {1 2 3}";
    let (raw, ty) = eval_src(src);
    // map sempre retorna List; se input era Array, codegen converte para Array.
    // O tipo na TAST é List(Int), mas verificamos que executa sem panic.
    assert!(
        matches!(ty, Ty::List(_) | Ty::Array(_)),
        "map sobre array deve retornar List ou Array, got {ty:?}"
    );
    assert_ne!(raw, 0, "resultado não-vazia");
}

/// `fold * 1 [1 2 3 4]` → 24 (fatorial: 1*1*2*3*4).
#[test]
fn fold_multiplicacao_retorna_24() {
    let src = "fold * 1 [1 2 3 4]";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "fold * deve retornar Int");
    assert_eq!(untag_smi(raw), 24, "fold * 1 [1 2 3 4] = 24");
}

/// `map (lambda x: + x 10) [0..1..5]` → map sobre Range.
#[test]
fn map_sobre_range_retorna_list() {
    let src = "map (lambda x: + x 10) [0..1..5]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "map sobre Range deve retornar List(Int)"
    );
    assert_ne!(raw, 0, "lista não-vazia");
    // Range [0..1..5] = 0,1,2,3,4 → +10 = 10,11,12,13,14
    assert_eq!(untag_smi(cons_head(raw)), 10, "primeiro elemento = 10");
}

/// `filter (lambda x: < x 3) [1 8 3 9]` → [1] (filtrar menores que 3).
#[test]
fn filter_menor_que_3_retorna_1() {
    let src = "filter (lambda x: < x 3) [1 8 3 9]";
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::List(Box::new(Ty::int())),
        "filter deve retornar List(Int)"
    );
    assert_ne!(raw, 0, "lista não-vazia (contém 1)");
    assert_eq!(untag_smi(cons_head(raw)), 1, "primeiro elemento = 1");
}
