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

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
        Command::Parse { file } => cmd_parse(&file),
        Command::Eval { expr } => cmd_eval(&expr),
        Command::Run { file } => cmd_run(&file),
    }
}

// ── Conversão de erros para miette::Report ──────────────────

/// Converte um erro que implementa `miette::Diagnostic` em `miette::Report`.
trait IntoReport {
    fn into_report(self) -> miette::Report;
}

impl<E: miette::Diagnostic + Send + Sync + 'static> IntoReport for E {
    fn into_report(self) -> miette::Report {
        miette::Report::new_boxed(Box::new(self))
    }
}

// ── Comandos ───────────────────────────────────────────────

fn cmd_lex(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(IntoReport::into_report)?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn cmd_parse(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(IntoReport::into_report)?;
    let module = parse(tokens).map_err(IntoReport::into_report)?;
    println!("{module:#?}");
    Ok(())
}

fn cmd_eval(expr: &str) -> miette::Result<()> {
    let result = run_pipeline(expr)?;
    print_result(&result);
    Ok(())
}

fn cmd_run(file: &str) -> miette::Result<()> {
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
fn run_pipeline(source: &str) -> miette::Result<ExecResult> {
    // 1. Lex
    let tokens = lex(source).map_err(IntoReport::into_report)?;

    // 2. Parse
    let module = parse(tokens).map_err(IntoReport::into_report)?;

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = load_prelude()
        .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
    let user =
        resolve(&module).map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
    let resolved = merge_resolved(prelude, user);

    // 4. Infer (typeck + dispatch)
    let typed = infer_module(&module, &resolved).map_err(IntoReport::into_report)?;

    // 5. Optimize (pass-through em Fio 1)
    let typed = optimize(typed);

    // 6. Codegen + JIT + executar
    let jit =
        jit_eval(&typed).map_err(|e| miette::Report::msg(format!("erro de codegen: {e:?}")))?;

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
        enum_registry: prelude.enum_registry,
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
fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
