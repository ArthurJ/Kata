//! Scanner de doctests — extrai blocos de teste de comentários multilinha.
//!
//! Pré-passo textual que escaneia o source por comentários `#{ }#`
//! contendo linhas `>>> `. O lexer descarta comentários completamente
//! (nenhum token, nenhum span), então este scanner opera no texto bruto
//! antes do pipeline normal.
//!
//! Cada `DocBlock` corresponde a uma sessão REPL isolada. Linhas `>>> `
//! consecutivas (sem linha vazia entre elas) compartilham a mesma sessão.
//! Uma linha vazia separa blocos — cada bloco começa sessão fresca.

/// Um caso individual de doctest: input + output esperado.
///
/// `expected` é `None` quando não há linhas de output esperado (ex:
/// declarações `constant`, `let`, `Sig` que não produzem output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocCase {
    /// Input exato passado para `ReplSession::handle`.
    /// Pode conter múltiplas linhas (input multiline).
    pub input: String,
    /// Output esperado, normalizado (trim trailing por linha).
    /// `None` significa "não produz output".
    pub expected: Option<String>,
    /// Linha do `>>> ` no source (1-indexed, para diagnósticos).
    pub line: usize,
}

/// Um bloco de doctest — sessão REPL isolada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocBlock {
    /// Casos: (input, expected_output)
    pub cases: Vec<DocCase>,
    /// Linha inicial no source (1-indexed, para diagnósticos).
    pub line: usize,
}

/// Escaneia source por comentários `#{ }#` contendo `>>> `.
/// Retorna lista de blocos de doctest.
///
/// O scanner:
/// 1. Itera sobre o source procurando `#{`
/// 2. Acumula conteúdo até `}#`
/// 3. No conteúdo, procura linhas `>>> `
/// 4. Se nenhuma `>>> ` → ignora (comentário normal)
/// 5. Se há `>>> ` → processa como doctest
pub fn scan_doctests(source: &str) -> Vec<DocBlock> {
    let mut blocks: Vec<DocBlock> = Vec::new();
    let mut chars = source.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        if c != '#' {
            continue;
        }
        // Verificar `#{`
        if chars.peek().map(|(_, c)| *c) != Some('{') {
            continue;
        }
        // Consumir `{`
        chars.next();

        // Acumular conteúdo até `}#`
        let content_start = i + 2; // após `#{`
        let mut content_end = content_start;
        let mut found_close = false;
        while let Some((j, c)) = chars.next() {
            if c == '}' && chars.peek().map(|(_, c)| *c) == Some('#') {
                content_end = j;
                chars.next(); // consumir `#`
                found_close = true;
                break;
            }
        }
        if !found_close {
            // Comentário não fechado — ignorar
            continue;
        }

        let content = &source[content_start..content_end];

        // Linha inicial do comentário (1-indexed)
        let line = source[..i].lines().count() + 1;

        // Processar conteúdo do comentário
        if let Some(block_list) = process_comment(content, line) {
            blocks.extend(block_list);
        }
    }

    blocks
}

/// Processa o conteúdo de um `#{ }#` e extrai blocos de doctest.
///
/// Retorna `None` se não há `>>> ` no conteúdo (comentário normal).
/// Retorna `Some(vec)` se há doctests — pode ser múltiplos blocos
/// separados por linhas vazias.
fn process_comment(content: &str, base_line: usize) -> Option<Vec<DocBlock>> {
    // Verificar se há `>>> ` no conteúdo
    let has_doctest = content.lines().any(|l| l.starts_with(">>> "));
    if !has_doctest {
        return None;
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut blocks: Vec<DocBlock> = Vec::new();

    // Estado do caso atual
    let mut current_input = String::new();
    let mut current_output = String::new();
    let mut has_output = false;
    let mut case_line = 0usize; // linha do `>>> ` do caso atual
    let mut block_start_line = 0usize;
    let mut in_doctest = false;
    let mut current_cases: Vec<DocCase> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_num = base_line + idx;

        if line.starts_with(">>> ") || (line.starts_with(">>>") && line.len() == 3) {
            // Nova linha `>>> ` — fecha caso anterior se existir
            if !current_input.is_empty() {
                let expected = if has_output {
                    Some(normalize_output(&current_output))
                } else {
                    None
                };
                current_cases.push(DocCase {
                    input: current_input.trim_end_matches('\n').to_string(),
                    expected,
                    line: case_line,
                });
                current_input.clear();
                current_output.clear();
                has_output = false;
            }

            if !in_doctest {
                in_doctest = true;
                block_start_line = line_num;
            }

            // Extrair input após `>>> ` (ou `>>>` vazio)
            let input_part = if line.len() > 4 { &line[4..] } else { "" };
            current_input.push_str(input_part);
            current_input.push('\n');
            case_line = line_num;
        } else if in_doctest {
            if line.is_empty() {
                // Linha vazia separa blocos (se input está completo)
                if !current_input.is_empty() && !is_input_incomplete(&current_input) {
                    // Flush caso
                    let expected = if has_output {
                        Some(normalize_output(&current_output))
                    } else {
                        None
                    };
                    current_cases.push(DocCase {
                        input: current_input.trim_end_matches('\n').to_string(),
                        expected,
                        line: case_line,
                    });
                    current_input.clear();
                    current_output.clear();
                    has_output = false;
                    // Flush bloco
                    if !current_cases.is_empty() {
                        blocks.push(DocBlock {
                            cases: std::mem::take(&mut current_cases),
                            line: block_start_line,
                        });
                    }
                    block_start_line = 0;
                    in_doctest = false;
                }
                // Se input está incompleto, linha vazia termina o bloco
                // multiline (igual ao REPL interativo) — não adiciona.
            } else {
                // Linha não-vazia sem `>>> `
                if current_input.is_empty() {
                    continue;
                }
                // Heurística de multiline — igual ao REPL interativo:
                // Se o input começa com match/enum/implements/interface,
                // linhas indentadas são continuação (mesmo se o parser
                // aceita o input parcial — ex: match com 1 braço).
                // Caso contrário, usa is_input_incomplete (<EOF> no parse).
                let first_token = current_input.split_whitespace().next().unwrap_or("");
                let multiline_indent =
                    matches!(first_token, "match" | "enum" | "implements" | "interface");
                let is_continuation = if multiline_indent {
                    line.starts_with(' ')
                        || line.starts_with('\t')
                        || line.trim_start().starts_with("lambda")
                        || line.trim_start().starts_with("λ")
                } else {
                    is_input_incomplete(&current_input)
                };

                if is_continuation {
                    current_input.push_str(line);
                    current_input.push('\n');
                } else {
                    has_output = true;
                    current_output.push_str(line);
                    current_output.push('\n');
                }
            }
        }
    }

    // Flush final
    if !current_input.is_empty() {
        let expected = if has_output {
            Some(normalize_output(&current_output))
        } else {
            None
        };
        current_cases.push(DocCase {
            input: current_input.trim_end_matches('\n').to_string(),
            expected,
            line: case_line,
        });
    }
    if !current_cases.is_empty() {
        blocks.push(DocBlock {
            cases: current_cases,
            line: block_start_line,
        });
    }

    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

/// Verifica se o input está incompleto (parse falha com `<EOF>`).
/// Reutiliza a mesma heurística do REPL interativo.
fn is_input_incomplete(input: &str) -> bool {
    let tokens = match kata_lexer::lex(input) {
        Ok(t) => t,
        Err(_) => return false,
    };
    match kata_parser::parse(tokens) {
        Ok(_) => false,
        Err(e) => {
            let msg = format!("{e}");
            msg.contains("<EOF>")
        }
    }
}

/// Normaliza output: trim de whitespace à direita em cada linha,
/// concatenado com `\n`.
pub fn normalize_output(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Captura de stdout via dup2/pipe ──────────────────────────

/// Captura o stdout de uma closure via redirecionamento de fd.
///
/// Cria um pipe, salva stdout original, redireciona stdout para o
/// pipe_write, executa a closure, restaura stdout original, lê o
/// conteúdo do pipe_read e retorna.
///
/// Unix-only — doctests são código de teste no kata-driver, não
/// afetam `kata repl`, `kata run`, ou `kata build`.
#[cfg(unix)]
pub fn capture_stdout<F: FnOnce() -> R, R>(f: F) -> String {
    use std::os::fd::AsRawFd;

    let stdout_fd = std::io::stdout().as_raw_fd();

    // Criar pipe
    let (pipe_read, pipe_write) = {
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert!(ret == 0, "pipe falhou");
        (fds[0], fds[1])
    };

    // Salvar stdout original
    let saved_stdout = unsafe { libc::dup(stdout_fd) };
    assert!(saved_stdout != -1, "dup falhou");

    // Flush stdout antes de redirecionar
    let _ = std::io::Write::flush(&mut std::io::stdout());

    // Redirecionar stdout para pipe_write
    unsafe {
        libc::dup2(pipe_write, stdout_fd);
    }
    // Fechar pipe_write no lado do pai (o filho herdou via dup2)
    unsafe {
        libc::close(pipe_write);
    }

    // Executar a closure
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    // Flush stdout (escreve no pipe) antes de restaurar
    let _ = std::io::Write::flush(&mut std::io::stdout());

    // Restaurar stdout original
    unsafe {
        libc::dup2(saved_stdout, stdout_fd);
        libc::close(saved_stdout);
    }

    // Ler conteúdo do pipe
    let mut captured = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(pipe_read, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n <= 0 {
            break;
        }
        captured.extend_from_slice(&buf[..n as usize]);
    }
    unsafe {
        libc::close(pipe_read);
    }

    // Propagar panic se houve
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }

    String::from_utf8_lossy(&captured).to_string()
}

/// Stub Windows — executa a closure sem capturar stdout.
///
/// No Windows, doctests com output esperado não podem comparar output
/// (não há captura de stdout portável sem `AllocConsole` + redirect).
/// Casos sem `expected` (só verificam que não dá erro) ainda funcionam.
#[cfg(not(unix))]
pub fn capture_stdout<F: FnOnce() -> R, R>(f: F) -> String {
    f();
    String::new()
}
