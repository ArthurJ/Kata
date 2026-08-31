# tests/probe-nested — Oráculos do PRD-exaustividade-aninhada

Oráculos E2E da **Fase 1** do `docs/PRDs/PRD-exaustividade-aninhada.md`
(§4.1, §5.1, §11). Alvo: `nested_exhaustiveness_e2e.rs` /
`nested_redundancy_e2e.rs` em `crates/kata-driver/tests/` — cópia
mecânica destes fontes.

**Procedência:** os 21 probes A–M/K/Rat são os fontes EXATOS medidos em
`f64eff8`/`b5e2d9e` (2026-08-30, dir. `/tmp/kata5-probe-nested/` apagado;
recuperados byte-exatos do backup `/home/arthur/kata5-probe-nested-backup/`).
`probeF_fora_dominio`, `probeF_parcial`, `RatUmOuDois_wildcard` e
`RatUmOuDois_zero` foram autorados nesta sessão a partir das linhas do §11
e controles do §8; `probeK_wide` é um extra do backup (repro do §10
#K-enum-payload, fora do §11).

## Como rodar

```bash
target/debug/kata build tests/probe-nested/probeA.kata -o /tmp/probeA  # AOT
target/debug/kata run   tests/probe-nested/probeA.kata                 # JIT
```

Baselines medidos estão em `results-acb6099/` (`.build.out/.err`,
`.run.out/.err` por probe; binários AOT não incluídos). Commit `acb6099`
é docs-only desde `b5e2d9e` — o binário mede o MESMO estado do compilador
que a linha de base do PRD. ⚠️ Há diff **não-commitado** em
`crates/kata-parser/src/patterns.rs` (unwrap `(p)` → Grouping): o binário
de `target/debug/` NÃO o inclui; se commitado, `probeK_deep_paren` muda
de estado (parse-error → parse-ok) e a linha de base deve ser revalidada.

## Linha de base (medida em `acb6099`, 2026-08-31)

| Probe | build | run | Diagnóstico/output hoje | Papel | Esperado final (PRD §11) |
|---|---|---|---|---|---|
| probeA | 0 | 0 | `tem true` — verde por cegueira | RED F2 | F2: `NonExhaustiveMatch` `["Some False"]` |
| probeB | 0 | **132** | SIGILL (trap `lower_clause_chain`) — checker deixou passar | RED F2 | F2: mesmo erro em compile (fim do SIGILL) |
| probeC | 0 | 0 | `tem true`/`tem false` | controle | verde |
| probeD | 0 | 0 | `tem true` | controle | verde |
| probeE | **101** | **101** | panic `helpers.rs:104` index out of bounds | RED F1 | F1: `ArityMismatch` gracioso |
| probeE2 | 1 | 1 | `type.redundant_clause` — **falso-positivo** | RED F1 | F2: verde, sem falso-positivo |
| probeF | 1 | 1 | `type.mismatch` (literal ≠ refined em pattern) | parcial F4 | F4: verde, `um`/`dois` nos 2 backends |
| probeF2 | 0 | 0 | `algum` | controle | verde |
| probeF_fora_dominio | 1 | 1 | `type.mismatch` (`0:` sobre `UmOuDois`) | parcial F4 | F4: `TypeMismatch` com literal na mensagem |
| probeF_parcial | 1 | 1 | `type.mismatch` (só `1:`, falta `2:`) | parcial F4 | F4: `NonExhaustiveMatch` `["2"]` |
| probeG | 0 | 0 | `positivo`/`zero ou negativo` | controle | verde (regressão) |
| probeH | 1 | 1 | `type.non_exhaustive_match` (guards entre cláusulas, inline) | parcial F3 | F3: verde com output correto |
| probeH_with | 1 | 1 | `type.non_exhaustive_match` (guards via `with`) | parcial F3 | F3: verde — as DUAS formas exigidas |
| probeJ | 0 | 0 | `tem true` — braço morto silencioso | RED F2 | F2: `RedundantClause` |
| probeJ2 | 0 | 0 | `tem` — otherwise inútil silencioso | controle | verde nas duas pontas (isenção) |
| probeK_arity_tuple | **101** | **101** | panic `helpers.rs:104` (2 patterns vs 1 param tupla) | RED F1 | F1: `ArityMismatch` gracioso |
| probeK_deep | 0 | 0 | `true dentro`/`false dentro` | controle | verde provado pós-F2 |
| probeK_deep_hole | 0 | 0 | `true dentro` — verde com buraco 3 níveis | RED F2 (flagship) | F2: `NonExhaustiveMatch` `["Some (Some False)"]` |
| probeK_deep_paren | 1 | 1 | `parse.unexpected_token` | limite sintático | parse error hoje e sempre (§10 #K-paren) |
| probeK_grid | 0 | 0 | `vv`/`ff` | controle | verde (regressão multi-param) |
| probeK_grid_partial | 1 | 1 | `type.non_exhaustive_match` (3 de 4 células) | RED hoje | motor IGUALA antes de estender |
| probeK_wide | 1 | 1 | `type.no_overload` na chamada `foo (Ok 5)` | extra (§10) | fora do PRD — repro #K-enum-payload |
| probeM | 0 | 0 | `zero` — verde por cegueira (`Ok 0` cobre Int?) | RED F2 | F2: `NonExhaustiveMatch` `["Ok _"]` |
| RatUmOuDois | 1 | 1 | `type.unbound_name` (`rational` não é pattern de folha) | RED F5 | F5: verde com output correto nos 2 backends |
| RatUmOuDois_wildcard | 0 | 0 | `algum` — wildcard não desce na folha | controle F5 | verde |
| RatUmOuDois_zero | 1 | 1 | `type.unbound_name` | controle F5 | F5: `TypeMismatch` com o literal na mensagem |

## Classificação por fase (§4.1)

- **RED F2** (verdes por cegueira): probeA, probeB, probeM, probeJ, probeK_deep_hole.
- **RED F1** (panic/falso-positivo): probeE, probeK_arity_tuple, probeE2.
- **Controles verdes**: probeC, probeD, probeF2, probeG, probeJ2,
  probeK_deep, probeK_grid, RatUmOuDois_wildcard.
- **Pariais de fase**: probeH, probeH_with — `non_exhaustive_match` até F3;
  probeF, probeF_fora_dominio, probeF_parcial — `type.mismatch` até F4.
- **F5**: RatUmOuDois (+ controles wildcard/zero).
- **Limite sintático**: probeK_deep_paren (§10 #K-paren).

Oráculos de virada F2+ carregam `#[ignore]` até a fase-alvo quando
migrarem para testes Rust (§4.1).

## Notas de medição

- Todos os 20 oráculos preservados do PRD reproduzem o estado documentado
  em `b5e2d9e` (20/20), outputs idênticos aos baselines do backup.
- probeB: `build` exit 0, `run` SIGILL 132 — o trap de defesa do codegen
  dispara em runtime porque o checker aceitou (PRD §2 bug #1).
- probeE/probeK_arity_tuple: o panic ocorre em AMBOS os backends
  (build=run=101, `helpers.rs:104`).
- Os probes K evitam `foo None`/`foo (Ok 5)` em chamadas quando a
  variante não é expressável como argumento (§10 #K-call, #K-enum-payload);
  chamam só células construíveis — a exaustividade é checada na
  declaração, independente das chamadas.
- Construção de Rational refined nos probes Rat: via construtor falível
  (`RatUmOuDois (rational 1)` → `Ok v`); ascription de non-literal para
  refined não existe hoje (skill kata-code-authoring).
- `lambda True True:` multi-param (10 testes, regressão 2-A) já existe
  em `crates/kata-inference/tests/lambda_match_inference.rs` — não
  precisa de probe aqui.