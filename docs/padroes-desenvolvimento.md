# Padrões de Desenvolvimento — Kata-Lang

## Estrutura de Código

### Limite de Linhas por Arquivo

- **Soft limit: 450 linhas.** Arquivos acima de 450 linhas são candidatos à
  análise de split. A decisão é por responsabilidade, não por tamanho — uma
  arquivo coeso com responsabilidade única não deve ser splitado.
- **Hard limit: 500 linhas.** Arquivos acima de 500 linhas excedem a capacidade
  de leitura num único passo. Split é esperado (governado por responsabilidade)
  ou escalado para Kanban se o arquivo for coeso e o split exigir refactor de
  design. Aplica-se a
  `.rs`, `.kata` de exemplo, e documentação longa.

### Testes Unitários

- **Testes unitários em arquivos separados do código de produção.** Para cada
  módulo `src/foo.rs`, os testes residem em `tests/foo_test.rs` (ou
  `src/foo/tests.rs` se a crate usar diretórios). Isso mantém o código de
  produção enxuto e força a API a ser testável por consumidores reais.

### Cross-Fio Test Runner

- **Test runner cross-fio que roda todos os fios em sequência.** Cada fio
  adiciona seus testes ao suite global, não apenas ao seu crate de teste
  isolado. Isso captura regressões horizontais (cross-cutting) que testes por fio
  não pegam.

---

## Diagnóstico de Erros

### Erros de Código Kata (reportados ao programador Kata)

- **Usam `miette` com spans corretos.** Erros léxicos, sintáticos, de tipos e de
  codegen reportados ao usuário final devem carregar o span do código fonte que
  os originou. Span errado ou ausente é bug.

### Erros Internos do Compilador (bugs nossos)

- **Usam informação útil para nós, não para o usuário final.** `thiserror` para
  definir variantes, `expect()` com mensagem descritiva em vez de `unwrap()`, e
  stack traces quando relevante. O alvo aqui é diagnosticar e corrigir
  rapidamente — não formatar para o programador Kata.
- **Erros internos não carregam `Span`.** Não há código do usuário para apontar.
  Se o compilador crasha em um invariante, o bug é nosso (ver I6 no manual).

### Códigos Namespaced (sem numéricos)

- **Erros usam códigos namespaced por domínio** (`type.mismatch`,
  `parse.unexpected_token`, `codegen.internal`), não códigos numéricos (`E103`).
  Códigos namespaced são auto-documentáveis e estáveis — adicionar um erro novo
  não exige renumerar existentes.

---

## Segurança

- **Proibido `unsafe`.** Não há necessidade no momento. Se surgir uma demanda
  legítima (ex: otimização crítica), o bloco deve ser aprovado explicitamente,
  isolado, e documentado com comentário `// SAFETY:`.

---

## API e Visibilidade

- **`pub(crate)` por padrão. `pub` só quando outra crate de fato precisa.** Isso
  força repensar acoplamento antes de expor APIs e mantém a superfície pública de
  cada crate mínima.

### `unwrap()` e `expect()`

- **Nada de `unwrap()` em produção.** Use `expect("razão")` ou propagação com
  `?`. `unwrap()` só é aceito em testes.
- `expect()` é preferível a `unwrap()` porque documenta o invariante que
  justifica a confiança.

---

## Commits

- **Conventional commits.** Formato: `tipo(escopo): descrição`. Tipos: `feat`,
  `fix`, `refactor`, `test`, `docs`, `chore`. Escopo: a crate afetada (`lexer`,
  `parser`, `resolution`, `inference`, `monomorph`, `escape`, `tree-shaking`,
  `codegen`, `optimizer`, `rt`, `driver`, `ast`, `diagnostics`, `comptime`).

---

## CI (Integração Contínua)

Obrigatório em todo push/PR:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. `cargo doc --no-deps --document-private-items`
5. **Execução dos exemplos:** rodar `kata run examples/*.kata` para todo
   arquivo em `examples/`. Exemplo que não compila ou não executa corretamente é
   falha de CI.

---

## Testes

### Testes de Regressão

- Todo bug vira um `.kata` em `examples/` + um `#[test]`. O teste roda o
  compilador contra o arquivo e verifica o comportamento esperado (seja
  compilação bem-sucedida, erro específico, ou saída de execução).

### Snapshots

- **Snapshots obrigatórios para fases do pipeline.** Convenção: todo PR que
  altera lexer, parser, resolution, inference, ou codegen **deve** atualizar
  snapshots. Ferramenta: `insta`.

### Testes de Propriedade

- **Propriedades para round-trip e invariantes.** Ex: `parse(print(ast)) == ast`,
  "nenhum input válido causa panic". Ferramenta: `proptest`.

### Testes de Compile-Fail

- **Testes que verificam rejeição de código inválido.** O compilador deve emitir
  erro (não panic, não aceitar silenciosamente) para código que viola as regras
  da linguagem. Ferramenta: `trybuild`.

### Typeck + Codegen no Mesmo Fio

- **Cada feature do typeck tem seu codegen no mesmo fio.** Se o typeck aprova,
  o codegen executa. Não aprovar no typeck o que o codegen não implementa —
  isso quebra o contrato fundamental "se compila, executa".

---

## Performance

### Benchmarks

- **Benchmarks de regressão em fases críticas.** Parsing de arquivos grandes,
  type-checking de módulos com muitas interfaces. Não precisam rodar no CI, mas
  devem estar disponíveis para execução manual antes de grandes refatorações.
  Ferramenta: `criterion`.

---

## Feature Flags

- **Feature flags para fases experimentais.** Se uma fase do pipeline está
  incompleta, usar `#[cfg(feature = "...")]` para que o build principal
  permaneça verde enquanto o desenvolvimento avança.

---

## Definition of Done

Cada fio tem como critério de aceite:

1. **Manual atualizado** se a implementação divergiu do PRD. O PRD é a decisão
   de design; o manual é a descrição da realidade. Quando divergem, o manual
   reflete a realidade final.
2. **Typeck + codegen consistentes** — se o typeck aprova, o codegen executa.
3. **Testes do fio + cross-fio passing.**
4. **Snapshots atualizados** se fases do pipeline foram alteradas.
5. **`pub(crate)` por padrão** — nenhuma API exposta sem justificativa.