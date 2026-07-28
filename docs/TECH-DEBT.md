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

## 3. Closure escape via return de função nomeada — SIGSEGV

**Estado:** Quando uma função nomeada retorna um lambda (ou hole) que captura
um parâmetro da função, o codegen não propaga as captures através do return.
O `alloc_capture_box` é chamado no call site da closure retornada, mas as
captures (parâmetros da função nomeada) não existem no escopo do caller.

**Cadeia de falha:**
1. `make_adder :: Int => (Int -> Int)` com `lambda n: + _ n` — o lambda
   captura `n` (parâmetro de `make_adder`)
2. `let add5 := make_adder 5` — o codegen vê `Closure` (chamada de função),
   não `Lambda` direto, então não registra captures em `closure_captures`
3. `add5 3` é chamado via `call_indirect` sem `box_ptr` — a função lambda
   compilada espera `box_ptr` como segundo param, recebe lixo → SIGSEGV

**Localização:** `crates/kata-codegen/src/lowering/expr.rs`, arm `Let`
(linhas 304-322) — só propaga captures quando o value é diretamente um
`Lambda`. Não propaga quando o value é uma `Closure` (chamada de função que
retorna lambda com captures).

**Também afeta:** Closures criadas com hole syntax (`+ _ n`) dentro de
actions. O `alloc_capture_box` não encontra as captures no `var_map` da
action.

**Impacto:** Alto. Qualquer closure que escapa via return de função nomeada
ou que é criada dentro de action e captura variável local crasha em runtime.
O exemplo `examples/closure_escape.kata` usa entry point direto (sem action
nem função nomeada) como workaround.

**Solução necessária:** A ABI de closures com captures precisa carregar o
`box_ptr` junto com o `fn_ptr` quando a closure escapa via return. Hoje o
`box_ptr` é alocado no call site, mas as captures só existem no escopo onde
a closure foi criada. Isto exige mudança na representação de closures em
runtime (par `(fn_ptr, box_ptr)` em vez de apenas `fn_ptr`).