mod cli;
mod codegen;
mod errors;
mod kata_rt;
mod lexer;
mod module_loader;
mod optimizer;
mod parser;
mod repl;
mod type_checker;

use clap::Parser;
use cli::{Cli, Commands};

fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    // Configurar o logger baseado na verbosidade (-v, -vv, -vvv)
    let log_level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    env_logger::Builder::new().filter_level(log_level).init();

    log::info!("Kata Compiler iniciado.");

    // Informar se flags de depuração estão ativas
    if cli.dump_tokens {
        log::info!("Flag ativada: --dump-tokens");
    }
    if cli.dump_ast {
        log::info!("Flag ativada: --dump-ast");
    }
    if cli.dump_tast {
        log::info!("Flag ativada: --dump-tast");
    }

    match &cli.command {
        Commands::Build {
            entrypoint,
            output,
            release,
        } => {
            log::info!("Comando: BUILD");
            log::info!("Entrypoint: {}", entrypoint);
            log::info!("Output: {:?}", output);
            log::info!("Release mode: {}", release);

            // Inicializa o Gerenciador de Modulos com acesso a raiz padrao da StdLib e pasta local
            let mut loader = module_loader::ModuleLoader::new(vec![
                std::path::PathBuf::from("src"), // Para achar 'core.prelude' etc
                std::env::current_dir().unwrap(), // Para achar imports do usuario locais
            ]);

            // Carrega o ambiente magico base que todos os arquivos precisam implicitamente
            let prelude_env = match loader.load_module("core", None) {
                Ok(env) => env,
                Err(e) => {
                    log::error!("Falha critica ao carregar o Prelude (StdLib): {}", e);
                    return Err(miette::miette!("Falha na instalacao da stdlib"));
                }
            };

            // Tenta ler o arquivo de entrada
            let source = std::fs::read_to_string(&entrypoint)
                .map_err(|e| miette::miette!("Falha ao ler o arquivo {}: {}", entrypoint, e))?;

            // 1. Lexer
            let tokens = match lexer::lex(&source, lexer::LexMode::File) {
                Ok(t) => t,
                Err(errs) => {
                    for err in errs {
                        let msg = err.to_string();
                        ariadne::Report::build(ariadne::ReportKind::Error, (entrypoint.clone(), err.span()))
                            .with_message("Erro Léxico")
                            .with_label(
                                ariadne::Label::new((entrypoint.clone(), err.span()))
                                    .with_message(&msg)
                                    .with_color(ariadne::Color::Red),
                            )
                            .finish()
                            .eprint((entrypoint.clone(), ariadne::Source::from(&source)))
                            .unwrap();
                    }
                    return Err(miette::miette!("Falha na análise léxica."));
                }
            };

            if cli.dump_tokens {
                println!("--- TOKENS ---");
                for (t, s) in &tokens {
                    println!("{:?} {:?}", t, s);
                }
            }

            // 2. Parser
            let source_len = source.chars().count();
            let module = match parser::parse_module(tokens.clone(), source_len) {
                Ok(m) => m,
                Err(errs) => {
                    for err in errs {
                        let msg = if let chumsky::error::SimpleReason::Unexpected = err.reason() {
                            format!(
                                "Token inesperado: {}",
                                err.found()
                                    .map(|t| t.to_string())
                                    .unwrap_or_else(|| "fim de arquivo".to_string())
                            )
                        } else {
                            err.to_string()
                        };

                        ariadne::Report::build(ariadne::ReportKind::Error, (entrypoint.clone(), err.span()))
                            .with_message(&msg)
                            .with_label(
                                ariadne::Label::new((entrypoint.clone(), err.span()))
                                    .with_message(msg.clone())
                                    .with_color(ariadne::Color::Red),
                            )
                            .finish()
                            .eprint((entrypoint.clone(), ariadne::Source::from(&source)))
                            .unwrap();
                    }
                    return Err(miette::miette!("Falha na análise sintática."));
                }
            };

            if cli.dump_ast {
                println!("--- AST PLANA ---");
                println!("{:#?}", module);
            }

            // Carrega qualquer outro import explicito que o arquivo Entrypoint faca
            let entrypoint_dir = std::path::Path::new(&entrypoint).parent();
            for (decl, _) in &module.declarations {
                if let parser::ast::TopLevel::Import(path, _) = decl {
                    if let Err(e) = loader.load_module(path, entrypoint_dir) {
                        log::error!("Erro de importacao: {}", e);
                        return Err(miette::miette!("Dependencia nao encontrada."));
                    }
                }
            }

            // 3. Type Checker (Arity Resolution & Types) do arquivo principal
            let mut checker = type_checker::Checker::new();
            checker.compiled_modules = loader.cache.clone();
            
            // Injeta o Prelude magicamente em todos os arquivos de Entrypoint compilados na raiz
            let all_exports: Vec<(String, Option<String>)> = prelude_env.exports.iter().map(|e| (e.clone(), None)).collect();
            checker.env.import_from(&prelude_env, "core", &all_exports);

            // Import_from nativo das dependencias locais do Entrypoint antes de validar
            for (decl, _) in &module.declarations {
                if let parser::ast::TopLevel::Import(path, specific) = decl {
                    if let Some(target_module_name) = path.split('.').next() {
                        if let Some(target_env) = checker.compiled_modules.get(target_module_name) {
                            checker.env.import_from(target_env, target_module_name, specific);
                        }
                    }
                }
            }

            checker.check_module(&module);

            if cli.dump_tast {
                println!("--- TAST (RESOLVIDA) ---");
                println!("{:#?}", checker.tast);
            }

            if !checker.errors.is_empty() {
                log::error!("Erros Semanticos detectados na Fase 3:");
                for e in &checker.errors {
                    log::error!("{}", e.0);
                }
                return Err(miette::miette!("Falha na analise semantica."));
            }

            // Precisamos compilar o TAST do proprio arquivo + a TAST de todas as dependencias de que ele precisa
            let mut full_tast = Vec::new();
            for (_, dep_tast) in loader.tast_cache {
                full_tast.extend(dep_tast);
            }
            full_tast.extend(checker.tast);

            let mut opt = optimizer::Optimizer::new(&checker.env);
            let optimized_tast = opt.optimize(full_tast, *release);

            if !opt.errors.is_empty() {
                log::error!("Erros de Otimizacao detectados na Fase 4:");
                for e in &opt.errors {
                    log::error!("{}", e.message);
                }
                return Err(miette::miette!("Falha na otimizacao."));
            }

            if cli.dump_tast {
                println!("--- TAST (OTIMIZADA) ---");
                println!("{:#?}", optimized_tast);
            }

            let out_bin = output.clone().unwrap_or_else(|| {
                std::path::Path::new(&entrypoint)
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string()
            });

            if let Err(e) = codegen::compile_and_link(optimized_tast, &checker.env, &out_bin) {
                log::error!("Erro de Compilacao/Codegen: {}", e);
                return Err(miette::miette!("Falha na geracao de codigo nativo."));
            }
            log::info!("Binario gerado com sucesso: {}", out_bin);
        }
        Commands::Run {
            entrypoint,
            release,
        } => {
            log::info!("Comando: RUN");
            log::info!("Entrypoint: {}", entrypoint);
            log::info!("Release mode: {}", release);

            let mut loader = module_loader::ModuleLoader::new(vec![
                std::path::PathBuf::from("src"),
                std::env::current_dir().unwrap(),
            ]);

            let prelude_env = match loader.load_module("core", None) {
                Ok(env) => env,
                Err(e) => return Err(miette::miette!("Falha critica ao carregar o Prelude: {}", e)),
            };

            let source = std::fs::read_to_string(&entrypoint)
                .map_err(|e| miette::miette!("Falha ao ler o arquivo {}: {}", entrypoint, e))?;

            let tokens = match lexer::lex(&source, lexer::LexMode::File) {
                Ok(t) => t,
                Err(_) => return Err(miette::miette!("Falha na análise léxica.")),
            };

            let source_len = source.chars().count();
            let module = match parser::parse_module(tokens, source_len) {
                Ok(m) => m,
                Err(_) => return Err(miette::miette!("Falha na análise sintática.")),
            };

            let entrypoint_dir = std::path::Path::new(&entrypoint).parent();
            for (decl, _) in &module.declarations {
                if let parser::ast::TopLevel::Import(path, _) = decl {
                    if let Err(e) = loader.load_module(path, entrypoint_dir) {
                        return Err(miette::miette!("Falha de dependencia: {}", e));
                    }
                }
            }

            let mut checker = type_checker::Checker::new();
            checker.compiled_modules = loader.cache.clone();
            
            let all_exports: Vec<(String, Option<String>)> = prelude_env.exports.iter().map(|e| (e.clone(), None)).collect();
            checker.env.import_from(&prelude_env, "core", &all_exports);

            for (decl, _) in &module.declarations {
                if let parser::ast::TopLevel::Import(path, specific) = decl {
                    if let Some(target_module_name) = path.split('.').next() {
                        if let Some(target_env) = checker.compiled_modules.get(path).or(checker.compiled_modules.get(target_module_name)) {
                            checker.env.import_from(target_env, target_module_name, specific);
                        }
                    }
                }
            }

            checker.check_module(&module);

            if !checker.errors.is_empty() {
                for e in &checker.errors { log::error!("{}", e.0); }
                return Err(miette::miette!("Falha na analise semantica."));
            }

            let mut full_tast = Vec::new();
            for (_, dep_tast) in loader.tast_cache { full_tast.extend(dep_tast); }
            full_tast.extend(checker.tast);

            let mut opt = optimizer::Optimizer::new(&checker.env);
            let optimized_tast = opt.optimize(full_tast, *release);

            if !opt.errors.is_empty() {
                for e in &opt.errors { log::error!("{}", e.message); }
                return Err(miette::miette!("Falha na otimizacao."));
            }

            let tmp_bin = ".tmp_kata_run";
            if let Err(e) = codegen::compile_and_link(optimized_tast, &checker.env, tmp_bin) {
                log::error!("Erro de Compilacao AOT: {}", e);
                return Err(miette::miette!("Falha na geracao de codigo nativo temporario."));
            }

            let mut child = std::process::Command::new(format!("./{}", tmp_bin))
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .map_err(|_| miette::miette!("Falha ao iniciar processo filho."))?;

            let status = child.wait().map_err(|_| miette::miette!("Falha ao aguardar processo."))?;

            // Expurgo
            let _ = std::fs::remove_file(tmp_bin);
            let _ = std::fs::remove_file(format!("{}.o", tmp_bin));
            let _ = std::fs::remove_file("kata_entry.c");

            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Commands::Test { path } => {
            log::info!("Comando: TEST");
            log::info!("Path: {}", path);

            // Tenta ler o arquivo
            let source = std::fs::read_to_string(&path)
                .map_err(|e| miette::miette!("Falha ao ler o arquivo de teste {}: {}", path, e))?;

            // 1. Lexer
            let tokens = match lexer::lex(&source, lexer::LexMode::File) {
                Ok(t) => t,
                Err(_) => return Err(miette::miette!("Falha na análise léxica do teste.")),
            };

            // 2. Parser
            let source_len = source.chars().count();
            let module = match parser::parse_module(tokens.clone(), source_len) {
                Ok(m) => m,
                Err(_) => return Err(miette::miette!("Falha na análise sintática do teste.")),
            };

            // 3. Type Checker
            let mut loader = module_loader::ModuleLoader::new(vec![
                std::path::PathBuf::from("src"),
                std::env::current_dir().unwrap(),
            ]);

            let prelude_env = match loader.load_module("core", None) {
                Ok(env) => env,
                Err(e) => return Err(miette::miette!("Falha ao carregar a stdlib no teste: {}", e)),
            };

            let mut checker = type_checker::Checker::new();
            checker.compiled_modules = loader.cache.clone();
            let all_exports: Vec<(String, Option<String>)> = prelude_env.exports.iter().map(|e| (e.clone(), None)).collect();
            checker.env.import_from(&prelude_env, "core", &all_exports);

            for (decl, _) in &module.declarations {
                if let parser::ast::TopLevel::Import(path, specific) = decl {
                    if let Err(e) = loader.load_module(path, Some(std::path::Path::new(&path))) {
                        return Err(miette::miette!("Falha de importacao no teste: {}", e));
                    }
                    if let Some(target_module_name) = path.split('.').next() {
                        if let Some(target_env) = loader.cache.get(target_module_name) {
                            checker.env.import_from(target_env, target_module_name, specific);
                        }
                    }
                }
            }

            checker.check_module(&module);

            log::info!("--- TESTES ENCONTRADOS ---");
            if checker.tests.is_empty() {
                log::info!("Nenhum bloco de teste anotado com @test encontrado.");
            } else {
                let mut compile_only_tests = Vec::new();
                let mut runtime_tests = Vec::new();

                for (i, t) in checker.tests.iter().enumerate() {
                    let tipo = if t.is_action { "[Action / Impuro]" } else { "[Função / Puro]" };
                    let expectation = if let Some(e) = &t.expects { format!(" (Expects: {})", e) } else { "".to_string() };
                    log::info!("{}) {} {} - \"{}\"{}", i + 1, tipo, t.name, t.description, expectation);

                    if t.expects.as_deref() == Some("CompileError") {
                        compile_only_tests.push(t.clone());
                    } else {
                        runtime_tests.push(t.clone());
                    }
                }

                let mut failed_count = 0;
                let mut passed_count = 0;

                // 1. Processa testes de Compilação (CompileError)
                for t in compile_only_tests {
                    log::info!("\nValidando Teste de Compilação: {}...", t.name);
                    let mut has_error = false;
                    for (err_msg, _) in &checker.errors {
                        if err_msg.to_string().contains(&t.name) {
                            has_error = true;
                            break;
                        }
                    }

                    if !has_error {
                        // Faz uma passagem rápida pelo otimizador só para ver se quebra em otimização de cauda ou comptime
                        let mut opt_checker = optimizer::Optimizer::new(&checker.env);
                        let mut stub_tast = Vec::new();
                        for (_, dep_tast) in loader.tast_cache.clone() { stub_tast.extend(dep_tast); }
                        stub_tast.extend(checker.tast.clone());
                        let _ = opt_checker.optimize(stub_tast, true);

                        for e in &opt_checker.errors {
                            // Apenas consideramos válido se o otimizador der *algum* erro, 
                            // já que em testes de Expected Failure o arquivo só tem essa intenção de erro.
                            has_error = true;
                            break;
                        }
                    }

                    if has_error {
                        log::info!("\\033[0;32mSUCESSO\\033[0m: Teste falhou na compilação conforme esperado.");
                        passed_count += 1;
                    } else {
                        log::error!("\\033[0;31mFALHA\\033[0m: Teste deveria ter gerado um CompileError, mas compilou limpo.");
                        failed_count += 1;
                    }
                }

                // 2. Processa testes de Runtime
                if !runtime_tests.is_empty() {
                    log::info!("\nGerando Entrypoint Sintético para {} teste(s) de runtime via Cranelift...", runtime_tests.len());

                    let mut test_body = Vec::new();
                    for test in &runtime_tests {
                        let call_span = 0..0;
                        
                        let echo_ident = (type_checker::tast::TExpr::Ident("echo".to_string(), parser::ast::TypeRef::Simple("Unknown".to_string())), call_span.clone());
                        let msg_literal = (type_checker::tast::TExpr::Literal(type_checker::tast::TLiteral::String(format!("Executando teste: {}...", test.name))), call_span.clone());
                        let echo_call = type_checker::tast::TStmt::Expr((type_checker::tast::TExpr::Call(Box::new(echo_ident), vec![msg_literal], parser::ast::TypeRef::Simple("()".to_string())), call_span.clone()));
                        test_body.push((echo_call, call_span.clone()));

                        if test.is_action {
                            let callee = Box::new((type_checker::tast::TExpr::Ident(test.name.clone(), parser::ast::TypeRef::Simple("Unknown".to_string())), call_span.clone()));
                            let call_expr = (type_checker::tast::TExpr::Call(callee, vec![], parser::ast::TypeRef::Simple("()".to_string())), call_span.clone());
                            test_body.push((type_checker::tast::TStmt::Expr(call_expr), call_span.clone()));
                        } else {
                            let callee = Box::new((type_checker::tast::TExpr::Ident(test.name.clone(), parser::ast::TypeRef::Simple("Unknown".to_string())), call_span.clone()));
                            let call_expr = (type_checker::tast::TExpr::Call(callee, vec![], parser::ast::TypeRef::Simple("Bool".to_string())), call_span.clone());
                            
                            let assert_ident = (type_checker::tast::TExpr::Ident("assert".to_string(), parser::ast::TypeRef::Simple("Unknown".to_string())), call_span.clone());
                            let assert_msg = (type_checker::tast::TExpr::Literal(type_checker::tast::TLiteral::String(format!("Falha matematica no teste puro: {}", test.name))), call_span.clone());
                            
                            let assert_call = type_checker::tast::TStmt::Expr((type_checker::tast::TExpr::Call(Box::new(assert_ident), vec![call_expr, assert_msg], parser::ast::TypeRef::Simple("()".to_string())), call_span.clone()));
                            test_body.push((assert_call, call_span.clone()));
                        }
                    }

                    let call_span = 0..0;
                    let echo_ident = (type_checker::tast::TExpr::Ident("echo".to_string(), parser::ast::TypeRef::Simple("Unknown".to_string())), call_span.clone());
                    let msg_literal = (type_checker::tast::TExpr::Literal(type_checker::tast::TLiteral::String(format!("\\033[0;32mRUNTIME FINALIZADO\\033[0m: Testes concluidos com exit code 0."))), call_span.clone());
                    let echo_call = type_checker::tast::TStmt::Expr((type_checker::tast::TExpr::Call(Box::new(echo_ident), vec![msg_literal], parser::ast::TypeRef::Simple("()".to_string())), call_span.clone()));
                    test_body.push((echo_call, call_span.clone()));

                    let test_main = type_checker::checker::TTopLevel::ActionDef(
                        "main".to_string(),
                        vec![],
                        (parser::ast::TypeRef::Simple("()".to_string()), 0..0),
                        test_body,
                        vec![]
                    );

                    let mut full_tast = Vec::new();
                    for (_, dep_tast) in loader.tast_cache {
                        full_tast.extend(dep_tast);
                    }
                    full_tast.extend(checker.tast);
                    full_tast.push((test_main, 0..0));

                    let mut opt = optimizer::Optimizer::new(&checker.env);
                    let optimized_tast = opt.optimize(full_tast, true); 
                    
                    let out_bin = ".tmp_kata_test";
                    if let Err(e) = codegen::compile_and_link(optimized_tast, &checker.env, out_bin) {
                        log::error!("Erro de Compilacao no Teste Runtime: {}", e);
                        return Err(miette::miette!("Falha ao compilar testes de runtime."));
                    }

                    log::info!("--- EXECUTANDO TESTES NATIVOS ---");
                    let status = std::process::Command::new(format!("./{}", out_bin))
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit())
                        .status()
                        .map_err(|_| miette::miette!("Falha ao invocar o binario de teste."))?;

                    let _ = std::fs::remove_file(out_bin);
                    let _ = std::fs::remove_file(format!("{}.o", out_bin));
                    let _ = std::fs::remove_file("kata_entry.c");

                    let has_expected_panic = runtime_tests.iter().any(|t| t.expects.as_deref() == Some("Panic"));
                    
                    if status.success() {
                        if has_expected_panic {
                            log::error!("\\033[0;31mFALHA GERAL\\033[0m: A suite rodou inteira, mas um teste esperava Panic.");
                            failed_count += runtime_tests.len();
                        } else {
                            passed_count += runtime_tests.len();
                        }
                    } else {
                        if has_expected_panic {
                            log::info!("\\033[0;32mSUCESSO (PANIC)\\033[0m: A execucao falhou conforme esperado (Panic interceptado).");
                            passed_count += runtime_tests.len();
                        } else {
                            log::error!("\\033[0;31mFALHA DE RUNTIME\\033[0m: Os testes falharam com erro de execucao (Panic Nao Esperado).");
                            failed_count += runtime_tests.len();
                        }
                    }
                }

                log::info!("\n=== RESULTADO FINAL: {} Passaram, {} Falharam ===", passed_count, failed_count);
                if failed_count > 0 {
                    return Err(miette::miette!("A suite de testes possui falhas."));
                }
            }
        }
        Commands::Repl => {
            log::info!("Comando: REPL");
            repl::start();
        }
    }

    Ok(())
}
