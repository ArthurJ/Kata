use std::path::Path;

use clap::Parser;
use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_rt as rt;

/// CLI do compilador Kata.
#[derive(Parser)]
#[command(name = "kata", version, about = "Compilador da linguagem Kata")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Análise léxica — imprime tokens com spans
    Lex { file: String },
    /// Análise sintática — imprime AST via Debug pretty-print
    Parse { file: String },
    /// Avalia expressão via JIT, imprime resultado
    Eval { expr: String },
    /// Compila e executa arquivo via JIT
    Run { file: String },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => {
            if let Err(e) = cmd_lex(&file) {
                eprintln!("erro: {e:?}");
                std::process::exit(1);
            }
        }
        Command::Parse { file } => {
            if let Err(e) = cmd_parse(&file) {
                eprintln!("erro: {e:?}");
                std::process::exit(1);
            }
        }
        Command::Eval { expr } => {
            if let Err(e) = cmd_eval(&expr) {
                eprintln!("erro: {e:?}");
                std::process::exit(1);
            }
        }
        Command::Run { file } => {
            if let Err(e) = cmd_run(&file) {
                eprintln!("erro: {e:?}");
                std::process::exit(1);
            }
        }
    }
}

// ── Comandos ───────────────────────────────────────────────

fn cmd_lex(file: &str) -> Result<(), Box<dyn std::fmt::Debug>> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn cmd_parse(file: &str) -> Result<(), Box<dyn std::fmt::Debug>> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;
    let module = parse(tokens).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;
    println!("{module:#?}");
    Ok(())
}

fn cmd_eval(expr: &str) -> Result<(), Box<dyn std::fmt::Debug>> {
    let result = run_pipeline(expr)?;
    print_result(&result);
    Ok(())
}

fn cmd_run(file: &str) -> Result<(), Box<dyn std::fmt::Debug>> {
    let source = read_source(file)?;
    let result = run_pipeline(&source)?;
    print_result(&result);
    Ok(())
}

// ── Pipeline ───────────────────────────────────────────────

/// Resultado da execução — valor bruto + tipo para display.
struct ExecResult {
    raw: i64,
    ty: Ty,
}

/// Executa o pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
fn run_pipeline(source: &str) -> Result<ExecResult, Box<dyn std::fmt::Debug>> {
    // 1. Lex
    let tokens = lex(source).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;

    // 2. Parse
    let module = parse(tokens).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = load_prelude().map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;
    let user = resolve(&module).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;
    let resolved = merge_resolved(prelude, user);

    // 4. Infer (typeck + dispatch)
    let typed =
        infer_module(&module, &resolved).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;

    // 5. Optimize (pass-through em Fio 1)
    let typed = optimize(typed);

    // 6. Codegen + JIT + executar
    let jit = jit_eval(&typed).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)?;

    Ok(ExecResult {
        raw: jit.raw,
        ty: jit.ty,
    })
}

/// Combina prelude + módulo do usuário em um ResolvedModule único.
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);

    // TypeEnv: prelude é o escopo base, user é filho.
    let type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    // Adiciona bindings do user no escopo filho.
    // (TypeEnv::with_parent cria escopo vazio com parent; bindings do user
    // são adicionados via define durante infer_module.)
    // Para Fio 1, user modules são só expressões (sem declarations próprias),
    // então o type_env do prelude já tem tudo.

    ResolvedModule {
        type_env,
        signatures,
    }
}

// ── Display ────────────────────────────────────────────────

/// Imprime o resultado da execução com untagging apropriado.
fn print_result(result: &ExecResult) {
    match &result.ty {
        Ty::Prim(PrimTy::Int) => {
            // SMI untag: se LSB=1, é SMI (val >> 1); se LSB=0, é ponteiro BigInt.
            let val = result.raw;
            let s = if val & 1 == 1 {
                // SMI
                (val >> 1).to_string()
            } else {
                // BigInt — chama kata_rt_bi_show para converter.
                unsafe {
                    let ptr = rt::kata_rt_bi_show(val);
                    let cstr = std::ffi::CStr::from_ptr(ptr);
                    cstr.to_string_lossy().into_owned()
                }
            };
            println!("{s}");
        }
        Ty::Prim(PrimTy::Float) => {
            // Float: raw é f64 reinterpretado como bits.
            let f = f64::from_bits(result.raw as u64);
            println!("{f}");
        }
        Ty::Prim(PrimTy::Text) => {
            // Text: raw é ponteiro para C string.
            unsafe {
                let cstr = std::ffi::CStr::from_ptr(result.raw as *const std::os::raw::c_char);
                println!("{}", cstr.to_string_lossy());
            }
        }
        Ty::Prim(PrimTy::Rational) => {
            // Rational: raw é ponteiro para BigRational (tipo opaque do runtime).
            let r_ptr = result.raw as *const std::ffi::c_void;
            unsafe {
                let ptr = rt::kata_rt_rat_show(r_ptr as *const _);
                let cstr = std::ffi::CStr::from_ptr(ptr);
                println!("{}", cstr.to_string_lossy());
            }
        }
        Ty::Sum(name) if name == "Boolean" => {
            // Boolean: 1 = True, 0 = False.
            println!("{}", if result.raw == 1 { "True" } else { "False" });
        }
        Ty::Unit => {
            println!("()");
        }
        _ => {
            // Fallback: imprimir valor bruto.
            println!("{}", result.raw);
        }
    }
}

// ── Helpers ────────────────────────────────────────────────

/// Lê o conteúdo de um arquivo.
fn read_source(path: &str) -> Result<String, Box<dyn std::fmt::Debug>> {
    let path = Path::new(path);
    std::fs::read_to_string(path).map_err(|e| Box::new(e) as Box<dyn std::fmt::Debug>)
}
