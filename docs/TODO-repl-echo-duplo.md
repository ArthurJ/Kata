# TODO — Eco duplo no REPL com syntax highlighting

**Data:** 2026-08-05
**Arquivos afetados:** `crates/kata-driver/src/highlight.rs`, `crates/kata-driver/src/repl.rs`
**Status:** Não resolvido

## Problema

Quando o syntax highlighting está ativo (`highlight_char` retorna `true` para edições), cada caractere digitado aparece duplicado no prompt: `55` em vez de `5`, `:env:env` em vez de `:env`, `echo!(5)echo!(5)` em vez de `echo!(5)`.

Sem highlighting (`highlight_char` retorna `false`), o eco duplo desaparece, mas as cores do input também.

## Causa raiz

O rustyline (testado v14 e v15) tem um conflito entre dois caminhos de renderização em `edit_insert`:

1. **Caminho trivial** (sem refresh): escreve apenas o caractere novo via `write_and_flush`. Não chama o highlighter. Usado quando `highlight_char` retorna `false`.

2. **Caminho de refresh** (com highlighter): faz `clear_old_rows` (`\r\x1b[K`) + reimprime prompt + linha highlighted + posiciona cursor. Usado quando `highlight_char` retorna `true`.

O eco duplo ocorre no caminho de refresh: o `clear_old_rows` faz `\r\x1b[K` para limpar a linha, mas o caractere recém-ecoado pelo terminal (antes do `\r\x1b[K`) não é limpo corretamente. O resultado é que o texto anterior permanece visível e o novo texto é impresso por cima, criando a duplicação.

O problema acontece tanto em PTY quanto em terminal real.

## O que já foi tentado

### 1. `highlight_char` retorna `true` (rustyline 14)
- **Resultado:** Cores ativas, eco duplo.
- **Causa:** O caminho trivial nunca é usado; toda tecla faz refresh completo. O `\r\x1b[K` não limpa o eco anterior.

### 2. `highlight_char` retorna `_forced` (rustyline 14)
- **Resultado:** Sem eco duplo, mas sem cores no input (o caminho trivial não chama o highlighter). O prompt tem cor da renderização inicial.
- **Causa:** O caminho trivial escreve apenas o caractere novo, sem refresh, sem re-renderização.

### 3. Desabilitar `HistoryHinter` (hint retorna `None`)
- **Resultado:** Resolveu o eco duplo que era causado pelas sugestões de histórico inline com ANSI, que confundiam o cálculo de largura do cursor.
- **Status:** Mantido. O hinter continua desabilitado.

### 4. `ColorMode::Enabled` (default) vs `ColorMode::Forced`
- **Resultado:** Sem diferença no eco duplo. `Forced` garante cores em terminais que não reportam capacidade, mas não afeta o eco.

### 5. `highlight_prompt` sem cor (Borrowed)
- **Resultado:** Sem diferença no eco duplo. O problema é no highlight do input, não do prompt.

### 6. Upgrade para rustyline 15 + `CmdKind`
- **Resultado:** Sem diferença. O `edit_insert` na v15 ainda tem a mesma estrutura: `!self.highlight_char(CmdKind::Other)` decide entre caminho trivial e refresh.
- **API:** `highlight_char(line, pos, kind: CmdKind)` onde `CmdKind` é `Other`, `MoveCursor`, `ForcedRefresh`. Retornar `true` para `Other` faz o rustyline usar o caminho de refresh (com highlighter) em vez do trivial.

### 7. PTY sem echo local
- **Resultado:** Eco duplo persiste mesmo com `ECHO=False` no PTY. O eco não é do terminal — é do próprio rustyline reimprimindo a linha sem limpar corretamente o conteúdo anterior.

## Diagnóstico

O problema é fundamental na forma como o rustyline faz o refresh incremental:

```
clear_old_rows:  \r\x1b[K          # deveria limpar a linha inteira
print:           prompt + line     # reimprime prompt + texto highlighted
position cursor: \r\x1b[NC         # move cursor N posições
```

O `\r\x1b[K` deveria limpar do início ao fim da linha. Mas o caractere digitado já foi escrito na tela (pelo caminho trivial ou pelo eco) **antes** do `\r\x1b[K` executar. O `\x1b[K` limpa do cursor ao fim, mas se o cursor está no início (`\r`), deveria limpar tudo. O fato de não limpar sugere que:

1. O `\x1b[K` não está sendo processado pelo terminal (improvável com `TERM=xterm`).
2. O caractere é ecoado **depois** do `\r\x1b[K` e antes da re-impressão (timing).
3. O rustyline está fazendo duas chamadas de refresh por tecla (uma trivial, uma completa).

## Alternativas a explorar

### A. Patchear o rustyline localmente
Modificar `edit_insert` ou `clear_old_rows` em `tty/unix.rs` para garantir que a linha seja limpa antes de re-imprimir. Por exemplo, adicionar um `\x1b[K` extra após `\r` ou usar `\x1b[2K` (limpar linha inteira) em vez de `\x1b[K` (limpar do cursor ao fim).

**Pró:** Controla exatamente o comportamento.
**Contra:** Mantém um fork local do rustyline.

### B. Migrar para reedline
`reedline` (do nushell) é uma alternativa ao rustyline com suporte a syntax highlighting em tempo real. É mais pesado, mas foi desenhado para isso.

**Pró:** Syntax highlighting nativo sem os problemas do rustyline.
**Contra:** Mudança de biblioteca, API diferente, mais dependências.

### C. Implementar REPL manual sem rustyline
Usar `std::io::stdin` em raw mode diretamente, sem rustyline. Implementar line editing, history, e highlighting manualmente.

**Pró:** Controle total sobre a renderização.
**Contra:** Muito trabalho, reinventa a roda.

### D. Highlight apenas na linha final (após Enter)
Manter `highlight_char` retornando `false` durante a digitação (caminho trivial, sem eco duplo) e aplicar highlight apenas quando o usuário pressiona Enter (refresh forçado). O input não tem cor enquanto digita, mas a linha final (no histórico) teria cor.

**Pró:** Simples, sem eco duplo.
**Contra:** Experiência inferior — o usuário não vê cores enquanto digita.

### E. Usar `crossterm` ou `termion` para renderização
Em vez de depender do rustyline para a renderização, usar `crossterm` ou `termion` para controlar o terminal diretamente e implementar o highlighting manualmente. O rustyline seria usado apenas para line editing e history, sem o highlighter.

**Pró:** Controle sobre a renderização sem abandonar o rustyline.
**Contra:** Duplicação de esforço, possível conflito entre rustyline e crossterm/termion.

### F. Investigar se o problema é específico do terminal do usuário
O eco duplo acontece no terminal do Arthur e no PTY. Testar em outros terminais (alacritty, kitty, gnome-terminal) para ver se é específico ou universal. Se for específico, pode ser uma configuração do terminal.

## Estado atual do código

- **rustyline 15** (upgrade de 14).
- `highlight_char` retorna `true` para `CmdKind::Other` e `CmdKind::ForcedRefresh` (cores ativas, eco duplo presente).
- `hint` retorna `None` (sem sugestões de histórico).
- `highlight_prompt` colorido (bold green para `kata> `, gray para `...`).
- `highlight` chama `highlight_line` (colorização do input via lexer).
- `ColorMode::Forced`.
- `eval_expr` com `retain` (expressões puras não ficam presas na sessão).

## Como reproduzir

```bash
cd ~/workspace/Kata5
cargo build -p kata-driver
./target/debug/kata repl
# Digitar: 5
# Ver: 55 (eco duplo)
# Digitar: echo!(5)
# Ver: echo!(5)echo!(5) (eco duplo)
```

## Próximos passos sugeridos

1. **Testar alternativa A** (patchear rustyline localmente) — menor esforço, maior controle.
2. Se A não funcionar, **testar alternativa B** (migrar para reedline).
3. Como fallback, **usar alternativa D** (highlight apenas na linha final) — garante experiência funcional sem eco duplo.