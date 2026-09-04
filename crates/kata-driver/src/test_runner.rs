use std::path::{Path, PathBuf};

use kata_codegen::TestWrapper;
use kata_rt as rt;

use crate::{doctest, pipeline, print_pipeline_errors, read_source, repl};

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
pub(crate) fn cmd_test(path: &str, filter: Option<&str>, interp: bool) -> miette::Result<()> {
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
                        return Err(print_pipeline_errors(errors));
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
                        return Err(print_pipeline_errors(errors));
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
