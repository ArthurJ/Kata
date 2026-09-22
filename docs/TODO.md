# TODO — Kata-Lang

Único arquivo de pendências. Atualizado 2026-09-21.
Itens resolvidos devem ser removidos — o histórico vive no git.

---

## Pendentes
### 🟡 Médio

#### `spawn!`: invariante de arena COW herdada não é enforced

O manual §5.2.2 (caso ~2020) argumenta que o filho em `spawn!` completa
antes de usar qualquer ponteiro herdado da arena do pai (COW via fork()).
Mas isso é **confiado**, não enforced: o filho executa uma Action
arbitrária — se o corpo dessa Action referencia valores capturados do
contexto do pai (closures, parâmetros não serializados de forma
independente), pode tocar ponteiros cuja COW-page ainda não foi
copiada/invalidada. O modelo de structured concurrency garante que o
pai espera o filho, mas não garante isolamento de leitura COW durante
a execução concorrente.

**Pergunta concreta:** o que impede um fiber-filho de ler uma estrutura
alocada na arena do pai antes do fork? O `marshal/mod.rs` serializa
args, mas closures/`with` bindings capturam por referência?

**Caminhos possíveis:** (a) enforcement: rejeitar closures com captures
em `spawn!` (typeck já sabe `captures.len()`); (b) deep-copy serializada
de TODO o estado que o filho pode tocar; (c) aceitar risco documentado
se invariante for verificável empiricamente.

#### Trampoline do scheduler engole erros (interp)

`interp_trampoline` (`csp.rs:212-218`) captura qualquer `InterpError`
(exceto `Return`), imprime no stderr, e retorna `0`. O scheduler vê `0`
como sucesso — o exit code do processo não reflete o erro. Toda
validação de erro gracioso do interp tem sua mensagem impressa mas não
propagada como exit code não-zero.

**Impacto:** tests E2E não podem confiar em exit code para detectar
falhas do interp (precisam inspecionar stderr).

**Caminho:** o trampoline retorna `i64` (não `Result`), alinhado à FFI
do scheduler. Mudar para propagar erro requer reformular a interface
trampoline/scheduler ou usar um canal lateral (e.g. célula
`Mutex<Option<InterpError>>` no `InterpCtx`).

### 🟢 Baixo

#### Considerar checkpoints do bumpalo para loops em fibers de vida longa

Arenas per-fiber usam bumpalo com liberação exclusivamente por reset
em massa (`kata_rt_arena_destroy` no epílogo da action). Um fiber de
vida longa (loop infinito, servidor, REPL persistente) acumula lixo
linearmente na arena: cada iteração aloca temporários, o reset só
ocorre quando o fiber termina — que pode ser nunca.

Bumpalo 3.17+ expõe `Bump::raw_checkpoint()` + `reset_to_raw_checkpoint()`:
checkpoint LIFO O(1), libera chunks alocados desde o checkpoint. Permite
"reset parcial" por iteração de loop sem dealloc individual e sem trocar
de alocador. Lockfile já está em 3.20.3.

**Limitação identificada:** resolve temporários de iteração, mas **não
resolve acumuladores arena-allocated mutados com `:=`** — o acumulador
novo é alocado após o checkpoint, o velho vira lixo. Modelo de escopo
único do Kata (`match`/`for`/`loop` não abrem escopo) complica: "valor
vivo na próxima iteração" não é o mesmo que `EscapeTarget::Caller`
(que é sobre caller da action, não próxima iteração). Seria necessário
um nível adicional de escape analysis: bindings mutados dentro de loop
são "Caller-relativo-ao-checkpoint-da-iteração".

**Caminhos possíveis:** (a) checkpoint por iteração apenas quando
escape analysis prova que nenhum binding do escopo externo é rebind
para valor arena-allocated na iteração — otimização conservadora que
só melhora o caso "acumulador é SMI" (já fora da arena); (b) acumular
em região separada pré-alocada no início do loop, temporários no
checkpoint (exige mover o acumulador entre regiões); (c) aceitar o
acúmulo linear quando o acumulador é arena-allocated, usar checkpoint
só para temporários.

**Depende de:** decisão sobre o que fazer com acumuladores arena-allocated
em loops — sem isso, o ganho real é marginal (SMI não toca arena).

#### Tree-shaking por instância de família polimórfica

O tree-shaking remove funções por **nome** — se uma função com overloads
polimórficas é alcançada, **todas** as instâncias expandidas sobrevivem,
mesmo as nunca chamadas com aquele tipo concreto. Ex: `mod :: Int
Instance("NonZero","Float") => Int` sobrevive mesmo se `mod` só é
chamado com `Instance("NonZero","Int")`.

**Impacto:** baixo. As overloads extras são inócuas em runtime (nunca
executadas), mas ocupam espaço no binário e tempo de compilação
(Cranelift compila cada uma). Só seria significativo com famílias
grandes (centenas de instâncias) e corpos pesados.

**Caminho:** propagar o tipo do argumento até o `collect_refs` do
tree-shaking para distinguir qual instância específica uma chamada
refere-se a, permitindo remover overloads não-usadas antes do codegen.

---

## Futuro

- **Sintaxe de action call: posicional vs nomeado** — discutir remover a
  forma posicional `f!(x, y)` e manter apenas `f!{a: x, b: y}` (dict
  nomeado). Motivação: diferenciação sintática entre funções (`f x y`,
  curried) e actions (`f!{a: x, b: y}`, nomeado) — hoje ambas usam
  parênteses, diferindo só pelo ``. A mudança eliminaria a ambiguidade
  Grouping/Tuple no path de action calls (4 pontos de normalização
  duplicados: action_call.rs, action_infer.rs, csp_concurrency.rs,
  log_builtins.rs). Contras: migração de ~416 calls posicionais e
  verbosidade para 1 arg (`echo!{msg: x}` vs `echo!(x)`). Alternativa:
  açúcar sintático para aridade 0 (`f!()`) e 1 (`f!(x)` ≡ `f!{param: x}`
  usando o primeiro param da action), mantendo `!{` obrigatório para 2+.
  Decisão pendente — precisa avaliar ergonomia vs consistência.

- **Centralizar normalização Grouping→Tuple** — mesmo sem decisão sobre a
  sintaxe de action call, a normalização `Grouping → Tuple de 1` está
  duplicada em 4 pontos (action_call.rs:168, action_infer.rs:141,
  csp_concurrency.rs:134, log_builtins.rs:153). Extrair para função única
  e chamar de todos os sites, evitando divergência silenciosa (o bug do
  SIGSEGV foi causado por exatamente esse tipo de divergência).

- **`select_arms_different_types`** — test placeholder em
  `kata-inference/tests/csp_typeck.rs:215`, depende de T0 unification.
  Corpo vazio, sem assertions.