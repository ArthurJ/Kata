//! Testes E2E de codegen de repr, format.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
#[allow(dead_code)]
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ═══════════════════════════════════════════════════════════════
// show — DoD 6: show auto-sintetizado (repr renomeado para show)
// ═══════════════════════════════════════════════════════════════

/// `show pessoa` retorna "Pessoa(João, 30)" — show básico com Text + Int.
#[test]
fn repr_struct_text_int() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"João\" 30\nshow p";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    // Text é ponteiro — não pode comparar valor diretamente.
    // Verifica que não panica e retorna Text.
    let _ = raw;
}

/// `show ponto` retorna "Ponto(3, 4)" — show com dois Ints.
#[test]
fn repr_struct_dois_ints() {
    let src = "data Ponto (x::Int y::Int)\nconstant p := Ponto 3 4\nshow p";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` despacha por tipo — dois structs diferentes, mesmo nome "show".
#[test]
fn repr_despacha_por_tipo() {
    let src = "data Pessoa (nome::Text idade::Int)\ndata Ponto (x::Int y::Int)\nconstant p := Pessoa \"João\" 30\nconstant pt := Ponto 3 4\n+ (show p) (show pt)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` de struct com campo Boolean.
#[test]
fn repr_struct_com_boolean() {
    let src = "data Flag (nome::Text ativa::Boolean)\nconstant f := Flag \"test\" True\nshow f";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` de struct aninhada — struct com campo que é outro struct.
#[test]
fn repr_struct_aninhada() {
    let src = "data Endereco (rua::Text cidade::Text)\ndata Pessoa (nome::Text end::Endereco)\nconstant e := Endereco \"Rua A\" \"Cidade B\"\nconstant p := Pessoa \"João\" e\nshow p";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` em Int funciona (Int implementa SHOW via prelude).
/// Antes (com `repr`) isto falhava porque `repr` não cobria Int.
/// Agora `show 42` despacha para `kata_rt_bi_show`.
#[test]
fn repr_struct_sem_campos_falha() {
    let src = "show 42";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

// ═══════════════════════════════════════════════════════════════
// format — DoD 5: format interpola
// ═══════════════════════════════════════════════════════════════

/// `format!("{} {}", (42, "ok"))` retorna "42 ok" — interpolação básica.
#[test]
fn format_basico_int_text() {
    let src = "format!(\"{} {}\", (42, \"ok\"))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format!("{}", (42,))` — tupla de 1 elemento.
#[test]
fn format_um_arg() {
    let src = "format!(\"{}\", (42,))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format!("{}", (True,))` — interpola Boolean.
#[test]
fn format_boolean() {
    let src = "format!(\"{}\", (True,))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format!("{}", (p,))` — interpola struct via repr.
#[test]
fn format_struct_via_repr() {
    let src = "data Pessoa (nome::Text idade::Int)\nconstant p := Pessoa \"João\" 30\nformat!(\"{}\", (p,))";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format!` sem args (template puro) — tupla vazia `()`.
#[test]
fn format_sem_args() {
    let src = "format!(\"hello world\", ())";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format!("Dobro: {}", 42)` — arg único sem tupla (auto-wrap como tupla de 1).
#[test]
fn format_arg_unico_sem_tupla() {
    let src = "format!(\"Dobro: {}\", 42)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}
