//! Testes E2E da Fase 9 — Complex: tipo numérico puro em Kata.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoDs 11-14: smart constructor, dispatch via interface NUM,
//! dispatch SHOW, e interoperabilidade Complex + Int.
//!
//! `Complex` é definido em `stdlib/complex.kata` com `data Complex (re::Float im::Float)`,
//! implementa NUM (+, -, *) e SHOW (show). Não usa FFI — todas as operações
//! são funções Kata puras com corpo lambda.

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

/// Prelude do complex.kata — concatenado inline no source de cada teste
/// porque `import` não é processado pelo resolution em testes E2E.
const COMPLEX_PRELUDE: &str = include_str!("../../../stdlib/complex.kata");

/// Constrói o source de teste: complex.kata (sem `import`) + expressão.
fn make_src(expr: &str) -> String {
    // Remove a linha `import core` — o prelude já é carregado via load_prelude().
    let prelude = COMPLEX_PRELUDE
        .lines()
        .filter(|l| !l.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{prelude}\n{expr}")
}

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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Executa o pipeline até inferência (sem JIT) — para verificar erros de typeck.
fn infer_src(src: &str) -> Result<kata_inference::TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved)
}

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
/// ATENÇÃO: inclui interface_registry.merge(user.interface_registry) —
/// essencial para dispatch via interface (NUM, SHOW) de tipos do usuário.
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

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        functions: user.functions,
        actions: user.actions,
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Lê uma C string (Text) do ponteiro retornado pelo JIT.
fn read_text(raw: i64) -> String {
    unsafe {
        let cstr = std::ffi::CStr::from_ptr(raw as *const std::os::raw::c_char);
        cstr.to_string_lossy().into_owned()
    }
}

/// Reinterpreta raw como f64 (bits).
fn raw_to_f64(raw: i64) -> f64 {
    f64::from_bits(raw as u64)
}

// ═══════════════════════════════════════════════════════════════
// DoD 11: `Complex 3.0 4.0` constrói via smart constructor
// ═══════════════════════════════════════════════════════════════

/// DoD 11: `Complex 3.0 4.0` constrói uma struct alocada na arena.
/// O raw é um ponteiro (não SMI — LSB=0 para ponteiros de heap).
#[test]
fn complex_smart_constructor_aloca_struct() {
    let src = make_src("Complex 3.0 4.0");
    let (raw, ty) = eval_src(&src);
    assert_eq!(
        ty,
        Ty::Struct("Complex".to_string()),
        "Complex 3.0 4.0 deve retornar Ty::Struct(\"Complex\")"
    );
    // Struct é alocada na arena — raw é ponteiro (não-zero, LSB=0).
    assert_ne!(raw, 0, "struct alocada não deve ser ponteiro nulo");
}

// ═══════════════════════════════════════════════════════════════
// DoD 12: `+ (Complex 1.0 2.0) (Complex 3.0 4.0)` → `Complex 4.0 6.0`
// ═══════════════════════════════════════════════════════════════

/// DoD 12: Soma de dois Complex despacha via iface++ no Score e produz
/// `Complex 4.0 6.0`. Verificamos via `show` que o resultado é `"(4.0 + 6.0i)"`.
#[test]
fn complex_soma_despacha_via_iface_num() {
    let src = make_src("show (+ (Complex 1.0 2.0) (Complex 3.0 4.0))");
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::text(), "show deve retornar Text");
    let s = read_text(raw);
    assert_eq!(
        s, "(4.0 + 6.0i)",
        "show (+ (Complex 1.0 2.0) (Complex 3.0 4.0)) deve ser \"(4.0 + 6.0i)\""
    );
}

// ═══════════════════════════════════════════════════════════════
// DoD 13: `show (Complex 3.0 4.0)` → `"(3.0 + 4.0i)"`
// ═══════════════════════════════════════════════════════════════

/// DoD 13: `show` despacha via interface SHOW e produz a representação
/// canônica `"(3.0 + 4.0i)"`.
#[test]
fn complex_show_despacha_via_iface_show() {
    let src = make_src("show (Complex 3.0 4.0)");
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::text(), "show deve retornar Text");
    let s = read_text(raw);
    assert_eq!(
        s, "(3.0 + 4.0i)",
        "show (Complex 3.0 4.0) deve ser \"(3.0 + 4.0i)\""
    );
}

// ═══════════════════════════════════════════════════════════════
// DoD 14: `+ (Complex 1.0 0.0) 5` → interoperabilidade NUM (Complex + Int)
// ═══════════════════════════════════════════════════════════════

/// DoD 14: Soma de Complex + Int despacha para a overload `+ :: Complex Int => Complex`
/// e produz `Complex 6.0 0.0`. Verificamos via `show` que é `"(6.0 + 0.0i)"`.
#[test]
fn complex_soma_com_int_interoperabilidade() {
    let src = make_src("show (+ (Complex 1.0 0.0) 5)");
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::text(), "show deve retornar Text");
    let s = read_text(raw);
    assert_eq!(
        s, "(6.0 + 0.0i)",
        "show (+ (Complex 1.0 0.0) 5) deve ser \"(6.0 + 0.0i)\""
    );
}

// ═══════════════════════════════════════════════════════════════
// Testes adicionais: subtração, multiplicação
// ═══════════════════════════════════════════════════════════════

/// `-` do Complex: `(3.0 + 4.0i) - (1.0 + 2.0i) = (2.0 + 2.0i)`.
#[test]
fn complex_subtracao() {
    let src = make_src("show (- (Complex 3.0 4.0) (Complex 1.0 2.0))");
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::text());
    let s = read_text(raw);
    assert_eq!(s, "(2.0 + 2.0i)");
}

/// `*` do Complex: (a+bi)(c+di) = (ac-bd) + (ad+bc)i.
/// (1.0 + 2.0i) * (3.0 + 4.0i) = (3-8) + (4+6)i = (-5.0 + 10.0i).
#[test]
fn complex_multiplicacao() {
    let src = make_src("show (* (Complex 1.0 2.0) (Complex 3.0 4.0))");
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::text());
    let s = read_text(raw);
    assert_eq!(s, "(-5.0 + 10.0i)");
}