use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_stdlib_for_tests;

fn infer_src(src: &str) -> Result<kata_inference::TypedModule, kata_diagnostics::MiddleError> {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    infer_module(&module, &prelude)
}

fn main() {
    let cases = [
        (
            "lambda 1 param (funciona)",
            "f :: Int => Int\nlambda x: - x 1\nf 5",
        ),
        (
            "lambda 2 params (- a b)",
            "f :: Int Int => Int\nlambda a b: - a b\nf 5 3",
        ),
        (
            "lambda 2 params (+ a b)",
            "f :: Int Int => Int\nlambda a b: + a b\nf 5 3",
        ),
        (
            "lambda 2 params hint",
            "f :: Int Int => Int\nlambda a b: - a b\nf 5 3",
        ),
        ("lambda 2 params inline apply", "(lambda a b: - a b) 5 3"),
        (
            "lambda 2 params 1 literal",
            "f :: Int Int => Int\nlambda a b: - a 1\nf 5 3",
        ),
    ];
    for (label, src) in cases {
        print!("\n=== {label} ===\n  src: {src}\n  ");
        match infer_src(src) {
            Ok(tmod) => println!("OK: entry ty = {:?}", tmod.entry.node.ty),
            Err(e) => println!("ERRO: {e:?}"),
        }
    }
}
