use std::path::Path;

use clap::Parser;
use kata_core::ty::Ty;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::ResolvedModule;

mod aot;
mod display;
mod doctest;
mod highlight;
mod imports;
mod pipeline;
mod repl;
mod test_runner;

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
    Eval {
        expr: String,
        /// Imprime a CLIF canônica antes da execução
        #[arg(long = "emit-ir")]
        emit_ir: bool,
        /// Usa interpretador tree-walking em vez de JIT
        #[arg(long = "interp")]
        interp: bool,
    },
    /// Compila e executa arquivo via JIT
    Run {
        file: String,
        /// Imprime a CLIF canônica antes da execução
        #[arg(long = "emit-ir")]
        emit_ir: bool,
        /// Usa interpretador tree-walking em vez de JIT
        #[arg(long = "interp")]
        interp: bool,
    },
    /// Descobre e executa testes `@test` em arquivo ou diretório
    Test {
        /// Arquivo `.kata` ou diretório com `*.kata` (recursivo)
        path: String,
        /// Filtra testes por substring na descrição
        #[arg(long)]
        filter: Option<String>,
        /// Usa interpretador tree-walking em vez de JIT
        #[arg(long = "interp")]
        interp: bool,
    },
    /// Compila programa Kata para executável nativo (AOT)
    Build {
        /// Arquivo `.kata` de entrada
        file: String,
        /// Path de saída (default: nome do arquivo sem extensão no cwd)
        #[arg(short, long)]
        output: Option<String>,
        /// Link dinâmico contra libkata_rt.so (default: estático)
        #[arg(long)]
        dynamic: bool,
    },
    /// REPL interativo — TypeEnv persistente + JIT fresco por expressão
    Repl {
        /// Usa interpretador tree-walking em vez de JIT
        #[arg(long = "interp")]
        interp: bool,
    },
    /// Inicia o servidor LSP (Language Server Protocol) em stdio
    Lsp,
}

fn main() -> miette::Result<()> {
    // Usar thread com stack maior (32 MB) para permitir recursão
    // tree-walking profunda do interpretador. O JIT tem TCO/TRMA e
    // não depende disso, mas o interp (sem TCO) precisa de stack
    // maior para exemplos recursivos moderados.
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // 32 MB
        .spawn(main_inner)
        .expect("falha ao criar thread main");
    result.join().expect("thread main panicked")
}

fn main_inner() -> miette::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
        Command::Parse { file } => cmd_parse(&file),
        Command::Eval {
            expr,
            emit_ir,
            interp,
        } => cmd_eval(&expr, emit_ir, interp),
        Command::Run {
            file,
            emit_ir,
            interp,
        } => cmd_run(&file, emit_ir, interp),
        Command::Test {
            path,
            filter,
            interp,
        } => test_runner::cmd_test(&path, filter.as_deref(), interp),
        Command::Build {
            file,
            output,
            dynamic,
        } => aot::cmd_build(&file, output.as_deref(), dynamic),
        Command::Repl { interp } => repl::cmd_repl(interp),
        Command::Lsp => cmd_lsp(),
    }
}

// ── LSP ────────────────────────────────────────────────────

/// Inicia o servidor LSP em stdio. Delega para `kata_lsp::run_stdio`.
fn cmd_lsp() -> miette::Result<()> {
    kata_lsp::run_stdio();
    Ok(())
}

// ── Conversão de erros para miette::Report ──────────────────

/// Converte um erro que implementa `miette::Diagnostic` em `miette::Report`
/// com `NamedSource` anexado, habilitando source context no miette
/// (linha de código + indicador de posição).
pub(crate) trait IntoReport: miette::Diagnostic + Send + Sync + Sized + 'static {
    /// Cria um `Report` com `NamedSource` anexado, habilitando source
    /// context no miette (linha de código + indicador de posição).
    ///
    /// - `source`: código-fonte completo do arquivo onde o erro ocorreu.
    /// - `file`: path do arquivo (ou `None` para eval/REPL → `<eval>`).
    fn into_report_with_source(self, source: &str, file: Option<&str>) -> miette::Report {
        let name = file.unwrap_or("<eval>");
        let named = miette::NamedSource::new(name, source.to_string());
        miette::Report::new_boxed(Box::new(self)).with_source_code(named)
    }
}

impl<E: miette::Diagnostic + Send + Sync + 'static> IntoReport for E {}

/// Formata um `Vec` de erros como string legível (um por linha).
/// `Vec<ResolveError>` não implementa `Display`, então `{e:?}` mostraria
/// `Debug` — incluindo spans brutos. Este helper usa `Display` de cada
/// erro individual, que delega para `thiserror::Error::fmt`.
pub(crate) fn format_error_vec<E: std::fmt::Display>(errors: &[E]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Converte `Vec<Report>` (erros de uma fase do pipeline) em um único
/// `miette::Report` para propagar em `miette::Result`.
///
/// - Se há exatamente 1 erro, retorna-o diretamente — o miette imprime
///   com o formato padrão (preservando snapshots existentes).
/// - Se há múltiplos erros, imprime cada um via `eprintln!` (sem o
///   prefixo `Error: ` que o `main` adicionaria) e retorna um `Report`
///   de resumo ("N erros encontrados").
pub(crate) fn print_pipeline_errors(errors: Vec<miette::Report>) -> miette::Report {
    if errors.len() == 1 {
        return errors
            .into_iter()
            .next()
            .expect("len == 1 garantido pelo if acima");
    }
    let n = errors.len();
    for report in &errors {
        eprintln!("{report:?}");
    }
    miette::Report::msg(format!("{n} erro(s) encontrado(s)"))
}

// ── Comandos ───────────────────────────────────────────────

fn cmd_lex(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn cmd_parse(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
    let module = parse(tokens).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
    println!("{module:#?}");
    Ok(())
}

fn cmd_eval(expr: &str, emit_ir: bool, interp: bool) -> miette::Result<()> {
    if interp {
        let result = run_pipeline_interp(expr, None)?;
        display::print_result(result.raw, &result.ty);
    } else {
        let result = run_pipeline_display_wrap(expr, emit_ir)?;
        display::print_result(result.raw, &result.ty);
    }
    Ok(())
}

fn cmd_run(file: &str, emit_ir: bool, interp: bool) -> miette::Result<()> {
    let source = read_source(file)?;
    let result = if interp {
        run_pipeline_interp(&source, Some(file))?
    } else {
        run_pipeline_with_file_display_wrap(&source, Some(file), emit_ir)?
    };
    // Unit de retorno de `main` não carrega informação — o output do
    // programa já foi produzido via echo!/_print!. Suprimir o `()`.
    if !matches!(result.ty, Ty::Unit) {
        display::print_result(result.raw, &result.ty);
    }
    Ok(())
}

// ── Pipeline ───────────────────────────────────────────────

/// Resultado da execução — valor bruto + tipo para display.
pub(crate) struct ExecResult {
    pub raw: i64,
    pub ty: Ty,
}

/// Igual a `run_pipeline` mas ativa display wrapping (show no entry point).
fn run_pipeline_display_wrap(source: &str, emit_ir: bool) -> miette::Result<ExecResult> {
    run_pipeline_with_file_display_wrap(source, None, emit_ir)
}

/// Executa o pipeline completo com caminho do arquivo (para resolver imports)
/// e display wrapping ativo (show no entry point).
///
/// Delega a sequência de compilação ao `Pipeline` composicional (A1).
/// Cada passo existe uma vez — este wrapper só escolhe os modos (two-pass,
/// tree-shake default) e termina com `jit_eval`.
fn run_pipeline_with_file_display_wrap(
    source: &str,
    file_path: Option<&str>,
    emit_ir: bool,
) -> miette::Result<ExecResult> {
    run_pipeline_with_file_inner(source, file_path, true, emit_ir)
}

fn run_pipeline_with_file_inner(
    source: &str,
    file_path: Option<&str>,
    display_wrap: bool,
    emit_ir: bool,
) -> miette::Result<ExecResult> {
    let mut pipeline =
        pipeline::Pipeline::new(source).with_file_path(file_path.unwrap_or("<eval>"));
    if display_wrap {
        pipeline = pipeline.with_display_wrap();
    }
    let compiled = (|| -> Result<_, Vec<miette::Report>> {
        pipeline
            .lex()?
            .parse(pipeline::ParseMode::TwoPass, file_path)?
            .resolve(file_path)?
            .desugar()
            .infer()?
            .monomorph()
            .optimize()
            .tree_shake(pipeline::ShakeMode::Default)?
            .comptime()?
            .build_type_table()
    })()
    .map_err(crate::print_pipeline_errors)?;

    let ty = compiled.entry_ty();
    let raw = compiled.jit_eval(emit_ir)?;

    Ok(ExecResult { raw, ty })
}

/// Executa via interpretador tree-walking (sem JIT/Cranelift).
///
/// Pipeline até `optimize()`, depois `Pipeline::interpret()`.
fn run_pipeline_interp(source: &str, file_path: Option<&str>) -> miette::Result<ExecResult> {
    let mut pipeline =
        pipeline::Pipeline::new(source).with_file_path(file_path.unwrap_or("<eval>"));
    pipeline = pipeline.with_display_wrap();

    let interp_module = (|| -> Result<_, Vec<miette::Report>> {
        pipeline
            .lex()?
            .parse(pipeline::ParseMode::TwoPass, file_path)?
            .resolve(file_path)?
            .desugar()
            .infer()?
            .monomorph()
            .optimize()
            .interpret()
    })()
    .map_err(crate::print_pipeline_errors)?;

    let ty = interp_module.entry_ty();
    let raw = interp_module.interp_eval()?;

    Ok(ExecResult { raw, ty })
}

/// Combina prelude + módulo do usuário em um ResolvedModule único.
/// Delega para `kata_resolution::merge_two` (compartilhado com ModuleLoader).
pub(crate) fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    kata_resolution::merge_two(prelude, user)
}

// ── Helpers ────────────────────────────────────────────────

/// Lê o conteúdo de um arquivo.
pub(crate) fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
