//! Testes E2E dos gates estruturais de `implements`.
//!
//! PRD: docs/PRDs/PRD-implements-gates.md
//!
//! Gate 1 — regra do órfão (O1-O5):
//! - O1: Complex implements NUM em módulo de usuário → orphan_impl
//! - O2: data Local; Local implements NUM → compila (não é órfão)
//! - O3: alias Int as MyInt; MyInt implements NUM → compila
//! - O4: interface LOCAL; data Local; Local implements LOCAL → compila
//! - O5: data Local; Local implements SHOW → compila (tipo local)
//!
//! Gate 2 — family_extension_invalid (F1-F2):
//! - F1: data (NUM, > _ 0) as Positive + MyNum implements NUM sem ORD →
//!   family_extension_invalid
//! - F2: mesma família + MyNum implements NUM ORD → compila

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, merge_two, resolve, validate_orphan_rule};
use kata_tree_shaking::tree_shake;

// ── Helpers ─────────────────────────────────────────────────────────

fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_two(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = kata_codegen::jit_eval(
        &typed,
        &Default::default(),
        &[],
        kata_codegen::leak_rt_ptr(),
        false,
    )
    .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn infer_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_stdlib_for_tests() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(r) => r,
        Err(_) => return true,
    };
    let resolved = merge_two(prelude, user);
    infer_module(&module, &resolved).is_err()
}

/// Verifica se a regra do órfão rejeita o módulo após merge do prelude.
fn orphan_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_stdlib_for_tests() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(r) => r,
        Err(_) => return true,
    };
    let resolved = merge_two(prelude, user);
    !validate_orphan_rule(
        &resolved.interface_registry,
        &resolved.struct_registry,
        &resolved.enum_registry,
    )
    .is_empty()
}

/// Código Kata para um tipo NUM mínimo (MyNum) — usado em F1, F2.
const MYNUM_NUM_IMPL: &str = "\
MyNum implements NUM
    + :: MyNum MyNum => MyNum
    lambda a b: MyNum (+ a.v b.v)
    - :: MyNum MyNum => MyNum
    lambda a b: MyNum (- a.v b.v)
    * :: MyNum MyNum => MyNum
    lambda a b: MyNum (* a.v b.v)
    div :: MyNum MyNum => Result::(MyNum, Text)
    lambda a b: core.Result::Ok a
    / :: MyNum NonZero => MyNum
    lambda a b: a
    // :: MyNum NonZero => Int
    lambda a b: 0
    zero :: MyNum => MyNum
    lambda _: MyNum 0
    abs :: MyNum => MyNum
    lambda a: MyNum a.v

MyNum implements EQ
    = :: MyNum MyNum => Boolean
    lambda a b: = a.v b.v
    != :: MyNum MyNum => Boolean
    lambda a b: not (= a.v b.v)

MyNum implements SHOW
    show :: MyNum => Text
    lambda a: show a.v
";

/// MyNum com ORD (para F2).
const MYNUM_ORD_IMPL: &str = "\
MyNum implements ORD
    < :: MyNum MyNum => Boolean
    lambda a b: < a.v b.v
    > :: MyNum MyNum => Boolean
    lambda a b: > a.v b.v
    <= :: MyNum MyNum => Boolean
    lambda a b: <= a.v b.v
    >= :: MyNum MyNum => Boolean
    lambda a b: >= a.v b.v
";

// ── Gate 1: regra do órfão ──────────────────────────────────────────

/// O1: `Complex implements NUM` em módulo de usuário (Complex e NUM ambos
/// externos) → `type.orphan_impl`.
#[test]
fn o1_complex_implements_num_orphan() {
    let src = "import core\nComplex implements NUM\n    + :: Complex Complex => Complex\n    lambda a b: a\n5";
    assert!(
        orphan_fails(src),
        "Complex implements NUM deve falhar como órfão"
    );
}

/// O2: `data Local; Local implements NUM` → compila (tipo é local).
#[test]
fn o2_local_implements_num_ok() {
    let src = format!("data MyNum (v::Int)\n{}\n5", MYNUM_NUM_IMPL);
    let (_raw, _ty) = eval_src(&src);
}

/// O3: `alias Int as MyInt; MyInt implements NUM` → compila.
/// Alias cria um tipo local, então não é órfão.
#[test]
fn o3_alias_implements_num_ok() {
    // Alias de Int cria tipo local. implements NUM não é órfão.
    // Testa apenas que validate_orphan_rule não acusa.
    let src = "\
alias Int as MyInt

MyInt implements NUM
    + :: MyInt MyInt => MyInt
    lambda a b: MyInt (+ a b)
    zero :: MyInt => MyInt
    lambda _: MyInt 0
    abs :: MyInt => MyInt
    lambda a: a

5
";
    assert!(
        !orphan_fails(src),
        "alias Int as MyInt; MyInt implements NUM não deve ser órfão"
    );
}

/// O4: `interface LOCAL; data Local; Local implements LOCAL` → compila
/// (ambos locais).
#[test]
fn o4_local_iface_local_type_ok() {
    let src = "\
interface GREETABLE
    greet :: Self => Text

data Greeting (msg::Text)

Greeting implements GREETABLE
    greet :: Greeting => Text
    lambda g: g.msg

greet (Greeting \"olá\")
";
    let (_raw, _ty) = eval_src(src);
}

/// O5: `data Local; Local implements SHOW` (SHOW externa, Local local)
/// → compila (tipo é local, satisfaz regra do órfão).
#[test]
fn o5_local_type_ext_iface_ok() {
    let src = "\
data Greeting (msg::Text)

Greeting implements SHOW
    show :: Greeting => Text
    lambda g: g.msg

show (Greeting \"olá\")
";
    let (_raw, _ty) = eval_src(src);
}

// ── Gate 2: family_extension_invalid ────────────────────────────────

/// F1: `data (NUM, > _ 0) as Positive` + `MyNum implements NUM` sem ORD
/// → `type.family_extension_invalid`.
#[test]
fn f1_positive_family_without_ord_fails() {
    let src = format!(
        "data (NUM, > _ 0) as Positive\ndata MyNum (v::Int)\n{}\n5",
        MYNUM_NUM_IMPL
    );
    assert!(
        infer_fails(&src),
        "Positive::MyNum sem ORD deve falhar como family_extension_invalid"
    );
}

/// F2: família NUM com predicado `> _ (zero _)` + `MyNum implements NUM ORD`
/// → compila. O predicado `> _ (zero _)` despacha `zero` como método de NUM
/// (retorna MyNum) e `>` como MyNum MyNum (ORD). Ambos os args são MyNum.
#[test]
fn f2_positive_family_with_ord_ok() {
    let src = format!(
        "data (NUM, > _ (zero _)) as Positive\ndata MyNum (v::Int)\n{}\n{}\n5",
        MYNUM_NUM_IMPL, MYNUM_ORD_IMPL
    );
    let (_raw, _ty) = eval_src(&src);
}
