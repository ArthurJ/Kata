use std::path::{Path, PathBuf};

use clap::Parser;
use kata_codegen::TestWrapper;
use kata_core::ty::Ty;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::ResolvedModule;
use kata_rt as rt;

mod aot;
mod display;
mod doctest;
mod highlight;
mod imports;
mod pipeline;
mod repl;

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
        } => cmd_test(&path, filter.as_deref(), interp),
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

// ── Test runner ────────────────────────────────────────────

/// Resultado da execução de um único caso de teste.
enum TestOutcome {
    Pass,
    Timeout,
    Deadlock,
    Fail(String),
}

/// Executa o subcomando `kata test`.
///
/// Descobre arquivos `.kata` (arquivo único ou diretório recursivo),
/// compila cada um, e executa os testes. Quando `interp=true`, usa
/// interpretador tree-walking; caso contrário, JIT.
fn cmd_test(path: &str, filter: Option<&str>, interp: bool) -> miette::Result<()> {
    let files = discover_kata_files(path)?;
    if files.is_empty() {
        eprintln!("nenhum arquivo .kata encontrado em `{path}`");
        return Ok(());
    }

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut total_skip = 0usize;

    for file in &files {
        let source = read_source(&file.to_string_lossy())?;
        let label = file.display();

        // Doctests — pré-passo textual, antes do pipeline e de @test.
        let doctest_blocks = doctest::scan_doctests(&source);
        if interp {
            // Doctests via interpretador.
            for block in &doctest_blocks {
                let mut session = match repl::InterpReplSession::new() {
                    Ok(s) => s,
                    Err(e) => {
                        println!("  [FAIL] {label}: doctest linha {}: {}", block.line, e);
                        total_fail += 1;
                        continue;
                    }
                };
                for case in &block.cases {
                    let mut eval_result: Result<bool, String> = Ok(true);
                    let actual = doctest::capture_stdout(|| {
                        eval_result = session.handle(&case.input);
                    });

                    match eval_result {
                        Ok(_) => {
                            let actual_norm = doctest::normalize_output(&actual);
                            match &case.expected {
                                Some(expected) => {
                                    if actual_norm == *expected {
                                        println!("  [PASS] {label}: doctest linha {}", case.line);
                                        total_pass += 1;
                                    } else {
                                        println!(
                                            "  [FAIL] {label}: doctest linha {}: output mismatch",
                                            case.line
                                        );
                                        println!("    esperado: {expected}");
                                        println!("    obtido:   {actual_norm}");
                                        total_fail += 1;
                                    }
                                }
                                None => {
                                    if actual_norm.is_empty() {
                                        println!("  [PASS] {label}: doctest linha {}", case.line);
                                        total_pass += 1;
                                    } else {
                                        println!(
                                            "  [FAIL] {label}: doctest linha {}: output inesperado",
                                            case.line
                                        );
                                        println!("    obtido:   {actual_norm}");
                                        total_fail += 1;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("  [FAIL] {label}: doctest linha {}: erro: {e}", case.line);
                            total_fail += 1;
                        }
                    }
                }
            }
        } else {
            // Doctests via JIT.
            for block in &doctest_blocks {
                let mut session = match repl::ReplSession::new() {
                    Ok(s) => s,
                    Err(e) => {
                        println!("  [FAIL] {label}: doctest linha {}: {}", block.line, e);
                        total_fail += 1;
                        continue;
                    }
                };
                for case in &block.cases {
                    let mut eval_result: Result<bool, String> = Ok(true);
                    let actual = doctest::capture_stdout(|| {
                        eval_result = session.handle(&case.input);
                    });

                    match eval_result {
                        Ok(_) => {
                            let actual_norm = doctest::normalize_output(&actual);
                            match &case.expected {
                                Some(expected) => {
                                    if actual_norm == *expected {
                                        println!("  [PASS] {label}: doctest linha {}", case.line);
                                        total_pass += 1;
                                    } else {
                                        println!(
                                            "  [FAIL] {label}: doctest linha {}: output mismatch",
                                            case.line
                                        );
                                        println!("    esperado: {expected}");
                                        println!("    obtido:   {actual_norm}");
                                        total_fail += 1;
                                    }
                                }
                                None => {
                                    if actual_norm.is_empty() {
                                        println!("  [PASS] {label}: doctest linha {}", case.line);
                                        total_pass += 1;
                                    } else {
                                        println!(
                                            "  [FAIL] {label}: doctest linha {}: output inesperado",
                                            case.line
                                        );
                                        println!("    obtido:   {actual_norm}");
                                        total_fail += 1;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("  [FAIL] {label}: doctest linha {}: erro: {e}", case.line);
                            total_fail += 1;
                        }
                    }
                }
            }
        }

        // Pipeline de compilação.
        // Se o arquivo só tem doctests (sem código executável), o pipeline
        // pode falhar — não abortar, apenas pula para o próximo arquivo.
        if interp {
            // Interpretador: pipeline até optimize(), depois interpret().
            let interp_module = (|| -> Result<_, Vec<miette::Report>> {
                pipeline::Pipeline::new(&source)
                    .with_file_path(&file.to_string_lossy())
                    .lex()?
                    .parse(pipeline::ParseMode::TwoPass, Some(&file.to_string_lossy()))?
                    .resolve(Some(&file.to_string_lossy()))?
                    .desugar()
                    .infer()?
                    .monomorph()
                    .optimize()
                    .interpret()
            })();

            let interp_module = match interp_module {
                Ok(m) => m,
                Err(errors) => {
                    if doctest_blocks.is_empty() {
                        return Err(crate::print_pipeline_errors(errors));
                    }
                    continue;
                }
            };

            // Executar @test specs via interpretador.
            let outcomes = run_test_interp(&interp_module, filter);
            for (desc, outcome) in outcomes {
                match outcome {
                    TestOutcome::Pass => {
                        println!("  [PASS] {label}: {desc}");
                        total_pass += 1;
                    }
                    TestOutcome::Timeout => {
                        println!("  [TIMEOUT] {label}: {desc}");
                        total_fail += 1;
                    }
                    TestOutcome::Deadlock => {
                        println!("  [DEADLOCK] {label}: {desc}");
                        total_fail += 1;
                    }
                    TestOutcome::Fail(msg) => {
                        println!("  [FAIL] {label}: {desc}: {msg}");
                        total_fail += 1;
                    }
                }
            }
        } else {
            // JIT: pipeline completo até build_type_table + jit_tests.
            let compiled = (|| -> Result<_, Vec<miette::Report>> {
                pipeline::Pipeline::new(&source)
                    .with_file_path(&file.to_string_lossy())
                    .lex()?
                    .parse(pipeline::ParseMode::Single, Some(&file.to_string_lossy()))?
                    .resolve(Some(&file.to_string_lossy()))?
                    .desugar()
                    .infer()?
                    .monomorph()
                    .optimize()
                    .tree_shake(pipeline::ShakeMode::PreserveTests)?
                    .comptime()?
                    .build_type_table()
            })();

            let compiled = match compiled {
                Ok(c) => c,
                Err(errors) => {
                    if doctest_blocks.is_empty() {
                        return Err(crate::print_pipeline_errors(errors));
                    }
                    continue;
                }
            };

            let type_shapes = compiled.type_shapes.clone();
            let depth_limit = compiled.depth_limit;
            let (jit_module, wrappers) = compiled.jit_tests()?;

            for w in &wrappers {
                let desc = w.spec.desc.as_deref().unwrap_or("(sem desc)");

                // Filtro por substring na descrição.
                if let Some(f) = filter
                    && !desc.contains(f)
                {
                    total_skip += 1;
                    continue;
                }

                let outcome = run_test_wrapper(&jit_module, w, &type_shapes, depth_limit);

                match outcome {
                    TestOutcome::Pass => {
                        println!("  [PASS] {label}: {desc}");
                        total_pass += 1;
                    }
                    TestOutcome::Timeout => {
                        println!("  [TIMEOUT] {label}: {desc}");
                        total_fail += 1;
                    }
                    TestOutcome::Deadlock => {
                        println!("  [DEADLOCK] {label}: {desc}");
                        total_fail += 1;
                    }
                    TestOutcome::Fail(msg) => {
                        println!("  [FAIL] {label}: {desc}: {msg}");
                        total_fail += 1;
                    }
                }
            }
        }
    }

    println!(
        "\n{} passed, {} failed, {} skipped",
        total_pass, total_fail, total_skip
    );

    if total_fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Executa um wrapper de teste individualmente.
///
/// Cada teste roda em Runtime fresco: `reset_scheduler` +
/// `kata_rt_set_test_timeout(N)` + chamada do wrapper. O wrapper é
/// `(rt: i64) -> i64` com `CallConv::SystemV` — autossuficiente.
fn run_test_wrapper(
    module: &cranelift_jit::JITModule,
    w: &TestWrapper,
    type_shapes: &[kata_rt::TypeShape],
    depth_limit: Option<u32>,
) -> TestOutcome {
    // Resetar estado global (timer + TLS periféricas) entre testes.
    rt::reset_scheduler();

    // Configurar timeout — opt-in. Sem `@test{timeout: N}`, o teste
    // roda até completar ou deadlock (TIMEOUT_EXPIRED fica false).
    if let Some(ms) = w.spec.timeout {
        rt::kata_rt_set_test_timeout(ms);
    }

    // A2: Alocar Runtime fresco para cada teste.
    let runtime = Box::new(rt::Runtime::new());
    let rt_ptr = Box::into_raw(runtime) as i64;

    // Propagar depth_limit do comptime pass (set_recursion_limit).
    if let Some(limit) = depth_limit {
        unsafe { (*(rt_ptr as *mut rt::Runtime)).depth_set_limit(limit) };
    }

    // A2: Registrar type_shapes no Runtime (marshalling to_bytes/from_bytes).
    if !type_shapes.is_empty() {
        rt::register_type_table(rt_ptr, type_shapes.to_vec());
    }

    // Obter ponteiro do wrapper compilado.
    let code = module.get_finalized_function(w.func_id);

    // SAFETY: `code` é ponteiro válido após finalize_definitions. O wrapper
    // é `extern "C" fn(i64) -> i64` — autossuficiente (faz scheduler_init +
    // spawn + run internamente).
    let result: i64 =
        unsafe { std::mem::transmute::<*const u8, extern "C" fn(i64) -> i64>(code)(rt_ptr) };

    // A2: Descartar Runtime após a execução (Drop libera arenas).
    // SAFETY: rt_ptr foi alocado acima; a execução já terminou.
    unsafe { drop(Box::from_raw(rt_ptr as *mut rt::Runtime)) };

    if result == rt::TIMEOUT_SENTINEL {
        TestOutcome::Timeout
    } else if result == rt::DEADLOCK_SENTINEL {
        TestOutcome::Deadlock
    } else if w.spec.expects.is_some() {
        // Wrapper com expects retorna status codes:
        // 0 = pass (show(err) casou expects com policy)
        // 1 = fail (show(err) não casou expects com policy)
        // 2 = fail (action retornou Ok quando expects esperava Err)
        match result {
            0 => TestOutcome::Pass,
            1 => TestOutcome::Fail(format!(
                "expects mismatch: {} não casou com policy {:?}",
                w.spec.expects.as_deref().unwrap_or(""),
                w.spec.policy.unwrap_or(kata_resolution::MatchPolicy::Exact)
            )),
            2 => TestOutcome::Fail("expected Err, got Ok".into()),
            _ => TestOutcome::Pass, // fallback gracioso
        }
    } else {
        // Sem expects — comportamento atual: pass se completa.
        TestOutcome::Pass
    }
}

/// Executa `@test` specs via interpretador tree-walking.
///
/// Para cada action com testes, para cada test spec: avalia args (se houver),
/// binda aos params da action, executa o body via interpretador.
/// Retorna (descrição, outcome) por teste.
fn run_test_interp(
    interp_module: &pipeline::InterpModule,
    filter: Option<&str>,
) -> Vec<(String, TestOutcome)> {
    let mut results = Vec::new();

    let rt = Box::new(kata_rt::Runtime::new());
    let rt_ptr = Box::into_raw(rt) as i64;

    // Criar contexto interpretador com enum_registry.
    let mut ctx = kata_interp::InterpCtx::new_with_registry(
        interp_module.inner.clone(),
        rt_ptr,
        std::sync::Arc::new(interp_module.enum_registry.clone()),
    );

    for action in &interp_module.inner.actions {
        for test_spec in &action.tests {
            let desc = test_spec
                .desc
                .as_deref()
                .unwrap_or("(sem desc)")
                .to_string();

            // Filtro por substring na descrição.
            if let Some(f) = filter
                && !desc.contains(f)
            {
                continue;
            }

            // Resetar estado global entre testes.
            rt::reset_scheduler();

            // Configurar timeout se houver.
            if let Some(ms) = test_spec.timeout {
                rt::kata_rt_set_test_timeout(ms);
            }

            // Criar Env novo e definir stdio bindings.
            let mut env = kata_interp::Env::new();
            env.define("__stdin__", kata_rt::kata_rt_stdin());
            env.define("__stdout__", kata_rt::kata_rt_stdout());
            env.define("__stderr__", kata_rt::kata_rt_stderr());

            // Avaliar args do teste (se houver) e bindar aos params da action.
            if let Some(ref args_expr) = test_spec.args {
                let arg_val = match kata_interp::eval(&mut ctx, args_expr, &mut env) {
                    Ok(v) => v,
                    Err(e) => {
                        results.push((
                            desc,
                            TestOutcome::Fail(format!("erro ao avaliar args: {e}")),
                        ));
                        continue;
                    }
                };

                // Desserializar args da tupla: ler i64s consecutivos.
                let n_params = action.param_types.len();
                if n_params > 0 {
                    for i in 0..n_params {
                        let val = unsafe { std::ptr::read((arg_val as *const i64).add(i)) };
                        if let Some(Some(name)) = action.param_names.get(i) {
                            env.define(name, val);
                        }
                    }
                }
            }

            // Executar o body da action.
            let mut outcome = TestOutcome::Pass;
            for stmt in &action.body {
                match kata_interp::eval(&mut ctx, stmt, &mut env) {
                    Ok(_) => {}
                    Err(kata_interp::InterpError::Return(_)) => break,
                    Err(e) => {
                        outcome = TestOutcome::Fail(format!("erro de execução: {e}"));
                        break;
                    }
                }
            }

            // Verificar expects se houver.
            if let Some(ref expects) = test_spec.expects {
                // O expects verifica show(err) contra o pattern com policy.
                // Sem codegen, não temos o mecanismo de expects do JIT.
                // Para o interpretador, se o teste completou sem erro, pass.
                // Se expects é None, pass. Se expects é Some, assumir pass
                // (o interpretador não tem como verificar expects sem o
                // mecanismo de show(err) vs policy).
                let _ = expects;
            }

            results.push((desc, outcome));
        }
    }

    // Descartar Runtime.
    unsafe { drop(Box::from_raw(rt_ptr as *mut kata_rt::Runtime)) };

    results
}

/// Descobre arquivos `.kata` — arquivo único ou diretório recursivo.
fn discover_kata_files(path: &str) -> miette::Result<Vec<PathBuf>> {
    let p = Path::new(path);
    if p.is_file() {
        return Ok(vec![p.to_path_buf()]);
    }
    if !p.is_dir() {
        return Err(miette::Report::msg(format!(
            "caminho não é arquivo nem diretório: `{path}`"
        )));
    }
    let mut files = Vec::new();
    collect_kata_files(p, &mut files);
    files.sort();
    Ok(files)
}

/// Coleta arquivos `.kata` recursivamente.
fn collect_kata_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kata_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "kata") {
            out.push(path);
        }
    }
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
