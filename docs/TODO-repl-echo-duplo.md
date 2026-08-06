# TODO — Eco duplo no REPL com syntax highlighting

**Data:** 2026-08-05
**Arquivos afetados:** `crates/kata-driver/src/highlight.rs`, `crates/kata-driver/src/repl.rs`
**Status:** ✅ Resolvido (2026-08-06)

## Problema

Quando o syntax highlighting está ativo (`highlight_char` retorna `true` para edições),
cada caractere digitado aparece duplicado no prompt: `55` em vez de `5`, `:env:env`
em vez de `:env`, `echo!(5)echo!(5)` em vez de `echo!(5)`.

## Causa raiz

O bug estava em `highlight_line` (`highlight.rs`), **não** no rustyline.

O lexer produz um token `Eof` sintético com `span.offset = 0, span.len = 0`. A função
`highlight_line` iterava sobre todos os tokens, incluindo o `Eof`, sem filtrar tokens de
span zero. O processamento do `Eof` regredia `last_end`:

1. Após processar o token `IntLit("5")` (offset=0, len=1), `last_end = 1`.
2. O token `Eof` (offset=0, len=0): `tok_end = 0`, `end = 0.min(1) = 0`, `last_end = 0`.
3. Após o loop, `last_end (0) < line.len() (1)` → `result.push_str(&line[0..])` reimprime
   o input inteiro sem cor.

Resultado: `\x1b[32m5\x1b[0m` (token correto) + `5` (resto re-impresso) = eco duplo.

O mesmo mecanismo afetava qualquer input: `echo!(5)` produzia
`\x1b[32mecho\x1b[0m...\x1b[32m)\x1b[0m` + `echo!(5)` (resto sem cor).

## Correção

Adicionado um guard em `highlight_line` que skipa tokens com `span.len == 0`:

```rust
for tok in &tokens {
    // Tokens sintéticos (Eof, Indent, Dedent, StmtSep) têm span zero
    // (offset e len = 0). Processá-los regrediria `last_end` e faria
    // o "resto da linha" reimprimir todo o input sem cor — eco duplo.
    if tok.span.len == 0 {
        continue;
    }
    // ...
}
```

## Verificação

- Captura PTY antes da correção: `\x1b[32m5\x1b[0m5\r\x1b[7C` (eco duplo)
- Captura PTY após a correção: `\x1b[32m5\x1b[0m\r\x1b[7C` (correto)
- 31/31 testes E2E do REPL passam
- 18/18 testes do driver passam

## Histórico de investigação

### Hipótese inicial (incorreta)

A hipótese inicial era que o rustyline calculava errado a largura visível da linha
highlighted (contando bytes ANSI). A função `width()` em `tty/mod.rs:122` ignora ANSI
escapes, e parecia ser a suspeita natural.

### Virada do diagnóstico

A captura PTY mostrou que o `5` extra aparecia **dentro** do buffer de uma única
chamada `refresh_line` — não era uma segunda escrita do rustyline. Isso apontou para
o `highlight()` retornando uma string com o caractere extra, e não para um problema
de renderização do rustyline.

Um teste isolado do lexer (`cargo run --example test_highlight`) revelou que o token
`Eof` tinha `offset=0, len=0`, e a simulação da lógica de `highlight_line` reproduziu
o eco duplo sem envolver o rustyline.

### Tentativas anteriores (preservadas para referência)

1. `highlight_char` retorna `true` (rustyline 14) — cores ativas, eco duplo
2. `highlight_char` retorna `_forced` (rustyline 14) — sem eco duplo, sem cores
3. Desabilitar `HistoryHinter` — resolveu eco duplo causado por sugestões inline
   com ANSI (problema diferente, mantido)
4. `ColorMode::Enabled` vs `Forced` — sem diferença
5. `highlight_prompt` sem cor — sem diferença
6. Upgrade para rustyline 15 + `CmdKind` — sem diferença
7. PTY sem echo local — eco duplo persiste

Nenhuma destas tentativas resolveu porque o problema não estava no rustyline —
estava no nosso `highlight_line` processando tokens sintéticos de span zero.