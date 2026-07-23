# Dívida Técnica — Kata5

Itens identificados como débito técnico real (não resolvidos, não decisões
de design). Cada item descreve o problema, o estado atual do código, e o
impacto.

## 1. ✅ `Effect` enum e campo `effect` removidos — RESOLVIDO 2026-07-23

**Resolução:** O enum `Effect` inteiro (todos os 4 variants: `Puro`, `IO`,
`Spawn`, `ChannelOp`) e o campo `effect` de `TypedExpr` foram completamente
removidos. Nenhum variant era consumido — zero `==`, `!=`, ou `matches!` sobre
o campo em qualquer crate. A distinção Action vs função pura é feita
estruturalmente via `ret_ty: Some` vs `None`, não por efeito. Cerca de 80
referências em ~30 arquivos foram removidas, incluindo construções de struct
literals, propagações, tipos de retorno de função (tríade `(ty, kind, effect)`
→ `(ty, kind)`), imports, testes e snapshots.

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