# Dívida Técnica — Kata5

Itens identificados como débito técnico real (não resolvidos, não decisões
de design). Cada item descreve o problema, o estado atual do código, e o
impacto.

## 1. `Effect::IO` declarado mas nunca produzido nem consumido

**Estado:** `Effect` tem 4 variants (`Puro`, `IO`, `Spawn`, `ChannelOp`).
`Spawn` e `ChannelOp` são produzidos em `action_call.rs` e `csp.rs` (fork!,
channel!, send, recv, select). `IO` **não é produzido em nenhum lugar** — zero
ocorrências de `Effect::IO` no workspace.

Além disso, **nenhum variant de `Effect` é consumido**: zero `==`, `!=`, ou
`matches!` sobre o campo `effect` em qualquer crate. O codegen não consulta
`effect` para qualquer decisão. O otimizador (TRMA, stream fusion) não
consulta. O monomorphizer apenas propaga mecanicamente (`expr.effect`).

**Contexto:** O ROADMAP planejava que Actions teriam `Effect::IO` (Fio 3) e
que o sistema de efeitos seria fonte de verdade para otimização. O caminho
tomado foi outro: o typeck distingue Actions de funções puras via `ret_ty:
Some` vs `None`, e o codegen trata `ActionCall` diferente de `Closure`. A
distinção estrutural substituiu a distinção por efeito.

**Impacto:** Baixo. `Effect::IO` é código morto — pode ser removido do enum
sem quebrar nada. O campo `effect` em si é carregado sem consumidores, mas
seu custo é um byte por `TypedExpr`. Se um otimizador futuro precisar
distinguir pureza sem analisar a AST, `effect` está lá como infraestrutura.

**Decisão pendente:** remover `Effect::IO` do enum (承认 que IO não é um
conceito usado), ou deixar como hook de extensão para um futuro sistema de
efeitos real?

## 2. `@test{expects: "CompileError"}` — parsing ok, execução não implementada

**Estado:** O parser aceita `expects: "CompileError: msg"` e o codegen cria
um placeholder `FuncId::from_u32(0)` (sem wrapper JIT). Mas o driver
(`kata-driver/src/main.rs:192-199`) detecta o prefixo `"CompileError:"` e
imprime `[PENDENTE]` com `total_skip += 1; continue;` — não compila
sub-módulo isolado, não verifica falha.

O teste E2E `test_expects_compileerror_adiado` em
`kata-driver/tests/test_runner_e2e.rs` está `#[ignore =
"sub-módulos isolados (C1) não implementados"]`.

**O que falta:** O design C1 (sub-módulos isolados) exige que o driver:
1. Extraia o sub-módulo a ser testado (o módulo referenciado por `expects`)
2. Compile-o isoladamente (lexer → parser → resolution → inference → codegen)
3. Verifique que a compilação falha com o erro esperado (substring match
   na mensagem de erro)
4. Reporte PASS se falhou com o erro esperado, FAIL se compilou ou falhou
   com erro diferente

**Impacto:** Médio. Sem isto, testes negativos (`@test{expects:
"CompileError: ..."}`) são silenciosamente pulados — o programador escreve o
teste, o runner diz `[PENDENTE]`, mas a validação nunca acontece. Um teste
negativo que deveria falhar (porque o código compila quando não deveria) passa
desapercebido.

**Dependência:** O design C1 não tem fase atribuída no ROADMAP. A
infraestrutura de parsing e placeholder existe desde Fio 14, mas a lógica de
execução nunca foi implementada.