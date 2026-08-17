# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-08-16.

Os docs `TODO-*.md` foram removidos (obsoletos ou resolvidos). Pendências vivem aqui.

---

## Débito Técnico

### 1. `@test{expects: \"CompileError\"}` — execução não implementada

**Estado:** Parser aceita, codegen cria placeholder, driver imprime `[PENDENTE]` e pula.

**O que falta (design C1 — sub-módulos isolados):**
1. Extrair o sub-módulo referenciado por `expects`
2. Compilá-lo isoladamente (lex → parse → resolve → infer)
3. Verificar que a compilação falha com o erro esperado (substring match)
4. Reportar PASS se falhou com o erro esperado, FAIL se compilou ou falhou com erro diferente

**Arquivos:** `crates/kata-driver/src/main.rs:234-242`, `crates/kata-codegen/src/lowering/test_runner.rs:68-72`, `crates/kata-driver/tests/test_runner_e2e.rs` (teste `#[ignore]`)

**Impacto:** Médio. Testes negativos são silenciosamente pulados — o programador escreve o teste, o runner diz `[PENDENTE]`, mas a validação nunca acontece.

### 2. `$` (Spread) — funcionalidade precisa de revisão

**Estado:** Parcialmente implementado. `expand_spread()` em `variant_construct.rs`
expande tupla literal em argumentos posicionais, mas só funciona quando o
argumento após `$` é `Expr::Tuple` literal. Não funciona com variáveis
(`let args := (1, 2, 3); f $ args` — `args` é `Expr::Ident`, não `Expr::Tuple`).
O contexto 2 (`$ (tuplo)` como callee standalone) não tem implementação —
falha com `UnboundName`. `Expr::Spread` existe na AST mas nunca é produzido
pelo parser (variant defensivo). Seção removida do sintaxe-mapa até revisão.

**O que decidir:**
- `$` deve funcionar com variáveis (não só literais)? Se sim, a expansão
  precisa mover da AST para o typeck (decidir por tipo, não por construtor)
- `$` como callee standalone deve ser implementado ou removido?
- Ou remover `$` inteiramente (nenhum caso prático funciona que `f a b c` resolva)?

**Arquivos:** `crates/kata-inference/src/infer/variant_construct.rs:250-299`,
`crates/kata-inference/src/infer/apply.rs:201`,
`crates/kata-ast/src/expr.rs:193` (`Expr::Spread` — defensivo),
`crates/kata-codegen/tests/spread_ascription_e2e.rs` (testes existentes)

**Impacto:** Baixo. `$` com literal é redundante (`f $ (1, 2, 3)` ≡ `f 1 2 3`).
Sem utilidade real até suportar variáveis.

### 3. Sintaxe de assinatura de Action deveria usar `->` em vez de `=>`

**Estado:** Pendente (decisão de design).

**Problema:** Hoje `=>` aparece em dois contextos distintos:

1. **Declaração de função pura:** `nome :: Int Int => Int` — `=>` separa
   "o que entra" de "o que sai" no nível léxico da declaração. Correto.
2. **Assinatura de Action:** `action f(x::Int) => Unit` — também usa `=>`,
   mas Action é um *tipo*, não uma declaração no mesmo sentido que função pura.

O caso que quebra a regra é `Action(Int) => Unit` — um *tipo* usando `=>`,
que é o separador de *declaração*. Se fosse `Action(Int) -> Unit`, a
distinção seria limpa: `=>` sempre declara (separador léxico de assinatura),
`->` sempre descreve um tipo de função transitável
(`A B -> C` dentro de uma assinatura maior, ou tipo de Action).

**Proposta:** Assinatura de action passa a usar `->`:

- `action f(x::Int) -> Unit` (declaração)
- `Action(Int) -> Unit` (tipo)

`=>` fica reservado exclusivamente para declaração de função pura
(`nome :: ... => ...`). `->` cobre todos os contextos onde a seta descreve
um *tipo* de função (pura como valor transitável ou action como tipo).

**Arquivos a alterar (estimativa):** parser (token `=>` vs `->` em posição
de action), inference (type checking de assinatura de action), manual
(`Kata-lang-manual.md`, `sintaxe-mapa.md`), testes snapshot.

**Status:** Analisar. Confirmação pendente — Arthur pediu para registrar
o TODO; decisão final requer revisão do impacto no parser e nos
snapshots existentes.

### 4. Byte string `b'...'` — lexer não aceita aspa simples

**Estado:** Gap no lexer. tmLanguage já cobre (preemptivamente).

**Problema:** O dispatcher do lexer (`crates/kata-lexer/src/dispatch.rs:165`)
só intercepta `b"` — `b' if peek == Some('"')`. Não há branch para
`b'`. O módulo `bytes.rs` documenta apenas `b"..."`. `b'Hello'` cai
no fallback `_ => lex_ident(lex, &start)`, produzindo `Ident("b")`
seguido de `TextLit("Hello'")` (a aspa simples abre string single).

**Correção:** Adicionar branch `b' if peek == Some('\'')` no dispatch,
chamando `lex_bytes_string` (ou variante que aceita `'` como delimitador).
O `lex_bytes_string` precisa parametrizar o delimitador de fechamento
(`"` vs `'`) — hoje hardcoded em `Some('"')` no match do conteúdo.

**Arquivos:** `crates/kata-lexer/src/dispatch.rs:165`, `crates/kata-lexer/src/bytes.rs`

**Impacto:** Baixo. `b"..."` já funciona; `b'...'` é paridade sintática
com strings normais (dupla e simples são equivalentes).

---

## Migração de Exemplos

### `parallel.kata` (Cluster 4)

**Estado:** `spawn!` totalmente implementado (parser, inference, codegen, runtime). 10 testes E2E passando. Falta apenas criar `examples/parallel.kata` migrando `examples/legacy/test_parallel.kata` (ajustar sintaxe `n :: Int` → `n::Int`).

### Tensor (Cluster 3)

**Estado:** `test_tensor_math.kata` não migrado. Bug intencional de dot com shapes incompatíveis — decisão de design pendente.

---

## Futuro
- Tensor/SIMD
- `@restart` (retry policy para Actions)