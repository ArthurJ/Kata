//! Testes E2E de codegen de repr, format, varargs (...).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Infere sem JIT — para verificar erros de typeck.
fn infer_src(src: &str) -> Result<kata_inference::TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved)
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
    let src = "data Pessoa (nome::Text idade::Int)\nlet p := Pessoa \"João\" 30\nshow p";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    // Text é ponteiro — não pode comparar valor diretamente.
    // Verifica que não panica e retorna Text.
    let _ = raw;
}

/// `show ponto` retorna "Ponto(3, 4)" — show com dois Ints.
#[test]
fn repr_struct_dois_ints() {
    let src = "data Ponto (x::Int y::Int)\nlet p := Ponto 3 4\nshow p";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` despacha por tipo — dois structs diferentes, mesmo nome "show".
#[test]
fn repr_despacha_por_tipo() {
    let src = "data Pessoa (nome::Text idade::Int)\ndata Ponto (x::Int y::Int)\nlet p := Pessoa \"João\" 30\nlet pt := Ponto 3 4\nstring_concat (show p) (show pt)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` de struct com campo Boolean.
#[test]
fn repr_struct_com_boolean() {
    let src = "data Flag (nome::Text ativa::Boolean)\nlet f := Flag \"test\" Boolean::True\nshow f";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `show` de struct aninhada — struct com campo que é outro struct.
#[test]
fn repr_struct_aninhada() {
    let src = "data Endereco (rua::Text cidade::Text)\ndata Pessoa (nome::Text end::Endereco)\nlet e := Endereco \"Rua A\" \"Cidade B\"\nlet p := Pessoa \"João\" e\nshow p";
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

/// `format "{} {}" (42, "ok")` retorna "42 ok" — interpolação básica.
#[test]
fn format_basico_int_text() {
    let src = "format \"{} {}\" (42, \"ok\")";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format "{}" (42,)` — tupla de 1 elemento.
#[test]
fn format_um_arg() {
    let src = "format \"{}\" (42,)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format "{}" (Boolean::True)` — interpola Boolean.
#[test]
fn format_boolean() {
    let src = "format \"{}\" (Boolean::True,)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format "{}" (pessoa)` — interpola struct via repr.
#[test]
fn format_struct_via_repr() {
    let src =
        "data Pessoa (nome::Text idade::Int)\nlet p := Pessoa \"João\" 30\nformat \"{}\" (p,)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

/// `format` sem args (template puro) — tupla vazia `()`.
#[test]
fn format_sem_args() {
    let src = "format \"hello world\" ()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::text());
    let _ = raw;
}

// ═══════════════════════════════════════════════════════════════
// varargs (...) — Type... desugara para Tuple<Type>
// ═══════════════════════════════════════════════════════════════

/// `...` em assinatura de Action desugara para Tuple<Int>.
/// O caller passa a tupla explicitamente: `soma_tupla!((1, 2, 3))`.
/// Nota: ActionCall envolve args em tupla externa. O typeck precisa
/// empacotar args em tupla interna quando o param é varargs. Por enquanto,
/// o teste só verifica que a assinatura desugara corretamente.
#[test]
fn varargs_em_action_desugara_para_tupla() {
    // A assinatura (Int...) deve desugarar para Tuple<Int>. O body da
    // Action é trivial (42). O erro de dispatch no call site é esperado
    // — a feature de empacotamento no call site ainda não está implementada.
    let src = "action soma_tupla (args::Int...) => Int\n    42\n42";
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "varargs deve desugarar na assinatura: {:?}",
        result.err()
    );
}

/// `...` em assinatura de Sig (função pura).
/// `echo_all :: Text... => Text` desugara para `echo_all :: (Tuple<Text>) => Text`.
/// O caller passa a tupla: `echo_all ("a",)` (tupla de 1 elemento com trailing comma).
#[test]
fn varargs_em_sig_desugara_para_tupla() {
    let src = "echo_all :: Text... => Text\n@ffi(\"kata_rt_string_concat\")\necho_all (\"a\",)";
    let result = infer_src(src);
    assert!(
        result.is_ok(),
        "varargs em sig deve desugarar: {:?}",
        result.err()
    );
}
