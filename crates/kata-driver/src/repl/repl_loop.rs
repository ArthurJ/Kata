//! REPL loop — rustyline I/O e multiline accumulation.
//!
//! Extraído de `repl/mod.rs` — separa a interação I/O (rustyline,
//! multiline heuristics, history) da lógica de sessão (pipeline,
//! bindings, snapshots).

use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;

use crate::highlight::KataHelper;
use crate::repl::ReplSession;

/// Executa o subcomando `kata repl`.
pub(crate) fn cmd_repl() -> miette::Result<()> {
    let mut session = ReplSession::new().map_err(miette::Report::msg)?;

    // Configurar rustyline com cores forçadas.
    //
    // ColorMode::Forced garante colorização mesmo em terminais que não
    // reportam capacidade de cor. O eco duplo que víamos antes era
    // causado pelo HistoryHinter (sugestões inline com ANSI), não pelo
    // ColorMode — agora o hinter retorna None.
    let config = rustyline::config::Builder::new()
        .color_mode(rustyline::config::ColorMode::Forced)
        .build();
    let mut rl = Editor::<KataHelper, DefaultHistory>::with_config(config)
        .map_err(|e| miette::Report::msg(format!("erro ao iniciar rustyline: {e}")))?;
    rl.set_helper(Some(KataHelper::default()));

    // Carregar histórico.
    let _ = rl.load_history(&session.history_path);

    println!("Kata REPL — digite :help para comandos, :quit para sair");

    loop {
        // Lê a primeira linha.
        let first = match rl.readline("kata> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("erro de leitura: {e}");
                break;
            }
        };

        // Comandos `:` são processados imediatamente (sem multiline).
        let trimmed = first.trim();
        if trimmed.starts_with(':') || trimmed.is_empty() {
            let _ = rl.add_history_entry(&first);
            match session.handle(&first) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => eprintln!("{e}"),
            }
            continue;
        }
        // Expressão: acumula linhas até o input ser completo.
        // Heurística multiline:
        //   1. Se o parse falha com "<EOF>", o input está incompleto —
        //      continuar lendo (ex: `lambda n:`, `match True`).
        //   2. Se a primeira linha é uma assinatura de função (`nome :: ... => T`)
        //      sem `@ffi`, ativar modo multiline — acumular até linha em
        //      branco (cláusulas lambda indentadas seguem).
        //   3. Se a primeira linha termina com `=>` (action sem tipo de
        //      retorno), body indentado pode seguir.
        //   4. Se a primeira linha inicia um bloco indentado — `match`,
        //      `enum`, `implements` — ativar modo multiline (break on
        //      non-indented line), igual à Sig.
        let first_trimmed = first.trim_end();
        let multiline_sig = first_trimmed.contains("::")
            && first_trimmed.contains("=>")
            && !first_trimmed.contains("@ffi");
        let multiline_action = first_trimmed.ends_with("=>");
        let first_token = first_trimmed.split_whitespace().next().unwrap_or("");
        let multiline_indent = matches!(first_token, "match" | "enum" | "implements" | "interface");

        let mut buffer = first.clone();
        let in_multiline = multiline_sig || multiline_action || multiline_indent;

        loop {
            if !in_multiline {
                // Verifica se o parse falha com <EOF> (input incompleto).
                if !ReplSession::is_input_incomplete(&buffer) {
                    break;
                }
            }

            // Lê próxima linha com prompt de continuação.
            match rl.readline("   ... ") {
                Ok(line) => {
                    if line.trim().is_empty() {
                        // Linha vazia termina o bloco multiline.
                        break;
                    }
                    buffer.push('\n');
                    buffer.push_str(&line);
                    // Se estávamos em modo multiline_sig ou multiline_indent
                    // e a nova linha não é indentada nem começa com
                    // `lambda`/`λ`, o bloco terminou.
                    if (multiline_sig || multiline_indent)
                        && !line.starts_with(' ')
                        && !line.starts_with('\t')
                        && !line.trim_start().starts_with("lambda")
                        && !line.trim_start().starts_with("λ")
                    {
                        break;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    buffer.clear();
                    break;
                }
                Err(ReadlineError::Eof) => {
                    break;
                }
                Err(e) => {
                    eprintln!("erro de leitura: {e}");
                    break;
                }
            }
        }

        let _ = rl.add_history_entry(&buffer);
        match session.handle(&buffer) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => eprintln!("{e}"),
        }
    }

    // Salvar histórico.
    let _ = rl.save_history(&session.history_path);
    Ok(())
}