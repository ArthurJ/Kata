# Manual Técnico: Kata-Lang

Abaixo encontra-se a documentação de operação e arquitetura do compilador Kata,
extraída do comportamento intrínseco do código-fonte. A arquitetura baseia-se
num princípio de Fundação de Contratos, onde regras (como erros estruturados,
FFI e diretivas) são estritamente definidas num eixo central, e a análise flui
sequencialmente desde uma gramática agnóstica no frontend até à emissão robusta
de código nativo no backend.

## Princípios de Design

Antes da descrição operacional, é essencial entender os princípios que guiam
todas as decisões de design da linguagem. Estes princípios são **invariantes** —
não são preferências ou convenções, são restrições fundamentais que qualquer
mudança na linguagem ou no compilador deve respeitar.

### I1. Notação Prefixa Estrita

A linguagem adota notação prefixa (`+ 1 1` em vez de `1 + 1`). Esta não é uma
escolha estética — é uma decisão arquitetural com consequões concretas:

- **Elimina precedência de operadores**: o parser é recursive-descent puro sem
  Pratt parsing. Não há tabela de precedência para manter.
- **Elimina ambiguidade léxica**: `+1` é um número positivo, `+ 1` é a função
  `+` aplicada a `1`. O lexer não precisa de contexto para decidir.
- **Uniformidade**: `+`, `soma`, `fatorial` são todos identificadores tratados
  identicamente pelo parser. Operadores não são especiais.

A notação prefixa provou ser mais simples de implementar e manter que a
notação infixa.

### I2. Sem `if` — Invariante Absoluta

A linguagem **nunca teve `if`** no design. Lógica condicional é expressa
exclusivamente via pattern matching e guards. Esta é uma invariante absoluta:

- Lógica condicional é expressa exclusivamente via pattern matching e guards.
- A ausência de `if` força modelagem via dados (ADTs) em vez de fluxo de controle
  ad-hoc.
- Pattern matching garante exaustividade (o typeck verifica); guards garantem
  fallback (`otherwise` é mandatário). `if` não oferece nenhuma dessas garantias.
- Esta restrição elimina uma classe inteira de bugs ("esqueci o else", "cobertura
  incompleta") que existe em linguagens com `if/else`.

### I3. Barreira Pure/Impure Física

A separação entre funções puras e actions impuras não é convencional — é física:

- **Sintaxe diferente**: Actions usam `!` na chamada, funções não.
- **Operadores diferentes**: `?` só existe em Actions; `return` e `;` só em
  Actions.
- **Restrições diferentes**: Actions não podem ser recursivas; funções não podem
  ter loops, `var`, ou `?`.
- **Typeck enforce**: o compilador rejeita código que viola a barreira em
  compile-time, não em runtime.

A barreira física reduz o espaço de estados que o typeck precisa cobrir. Sem
ela, o compilador teria que rastrear pureza taint analysis — significativamente
mais complexo.

### I4. Funções são SEMPRE Puras

Toda função definida com `lambda`/`λ` é **matematicamente pura** — sem exceções.
Mesmo funções `@ffi` declaradas com assinatura de função (não action) são
tratadas como puras pelo compilador. Se a função C subjacente tiver efeitos
colaterais, é **responsabilidade do desenvolvedor** declará-la como Action (com
`!`), não do compilador inferir纯idade.

O otimizador confia nesta invariante: funções puras podem ser eliminadas por
tree shaking se não são alcançadas, reordenadas, e avaliadas em compile-time via
`@comptime`. Violar esta invariante (declarar função impura como pura) produz
comportamento indefinido.

### I5. Entry Point Implícito

Programas Kata **não exigem** `action main` ou `main!()`. O entry point é a
**última expressão top-level** do arquivo executado como entrypoint. `@comptime`
top-level (sem `action`) produz `HeapSnapshot` e não gera código de runtime.

Não assumir padrões de uso ao diagnosticar bugs — um arquivo Kata válido pode
ser apenas uma expressão (`+ 1 2`), ou um conjunto de definições seguidas de uma
expressão final.

### I6. Erros do Compilador ≠ Erros do Usuário

O sistema de diagnostics distingue duas categorias:

- **Erros do usuário** (sintaxe, tipos, resolução): sempre carregam `Span`
  apontando para o código-fonte. São reportados com `miette` colorido.
- **Erros internos do compilador** (bugs nossos, invariantes violadas): não
  carregam `Span` — não há código do usuário para apontar. Usam `expect()`
  com mensagens descritivas.

Não tentar adicionar `Span` a erros internos. Se o compilador crasha em um
invariante, o bug é nosso, não do código do usuário.

### I7. Orphan Rule e Span

`kata-core` depende de `kata-ast` (leaf crate) para ter acesso a `Span`. A
orphan rule do Rust (E0117) proíbe `impl From<ErrorA> for ErrorB` quando ambos
os tipos estão em crates diferentes do crate onde o `impl` está. A solução é
`.map_err(fn)` para conversão explícita, não `impl From`.

### I8. Sem Dependências Externas Pesadas

O compilador usa Cranelift (não LLVM) como backend. A razão: Cranelift é
auto-contido, sem dependências de sistema (LLVM exige instalação externa, libs
C, e tem API instável entre versões). Esta foi uma decisão essencial validada
— a fragilidade do build com LLVM tornava o
desenvolvimento impraticável.

## Convenções Léxicas Básicas

### Comentários

Apenas comentários de linha: `#` até o final da linha. Não existem blocos de
documentação (`///`, `"""doc"""`, `/** */`) na linguagem — se necessário no
futuro, a decisão de sintaxe pode ser tomada com mais contexto. Adiar é
seguro porque adicionar doc comments depois é estritamente aditivo.

```kata
# Isto é um comentário
let x := 42  # comentário ao lado do código
```

### Identificadores

Qualquer caractere não-reservado pode compor um identificador. (`+`, `-`, `*`,
`/`, `<`, `>`) são identificadores válidos usados como nomes de função — são
definidos na stdlib, não no compilador Rust. Esta uniformidade é consequência
direta da notação prefixa (I1): operadores não recebem tratamento especial no
parser.

### Literais Numéricos

Inteiros podem usar `_` como separador visual (sem significado semântico) e ser
representados em múltiplas bases:

```kata
42        # decimal implícito (padrão)
1_000     # separador visual
0xFF      # hexadecimal
0o77      # octal
0b1010    # binário
0d42      # decimal explícito
```

Inteiros têm precisão arbitrária (BigInt). A representação interna usa SMI
tagging transparente ao compilador — todo o pipeline vê `i64`; o runtime decide
se o valor cabe num SMI inline ou precisa de heap allocation.

Floats aceitam notação decimal e científica:

```kata
3.14
1.5e10
1.5E-10
```

`NaN` não existe na linguagem — o sistema de tipos bloqueia estaticamente
operações que resultariam em NaN.

### Rational

Rational é um número racional de precisão arbitrária
(`BigRational`: `BigInt` / `BigInt`). Diferente de Float, **toda operação é
exata** — `1/3 * 3 = 1`, não `0.999...`. Não há rounding, não há erro de runtime.

Literais Rational usam ascription de tipo `::Rational` (rebaixamento de
literal — ver §4.2.7):

```kata
3.14::Rational    # racional exato a partir do texto bruto
42::Rational      # inteiro como racional
0.5::Rational     # meio
```

O texto bruto do literal é preservado até o type checker — `3.14::Rational` não
passa por `f64`, então não há imprecisão na conversão.

Aritmética `+ - * /` é sempre exata. `@associative` é legítimo em `+` e `*`
(sem rounding quebra associatividade).

`show` imprime decimal quando o denominador é da forma `2^a · 5^b` (ex: `0.5`,
`0.25`, `3.14`); caso contrário imprime como fração (`1/3`, `5/6`):

```kata
show (0.5::Rational)         # "0.5"
show (1::Rational / 3::Rational)  # "1/3"
```

Conversões com Float são **explícitas** — sem coerção implícita:

```kata
to_float :: Rational => Float       # você sabe que perde precisão
from_float :: Float => Rational     # racional mais próximo do f64
```

### Strings

Três formas de string, seguindo o modelo Python sem r-strings nem f-strings:

```kata
"linha com escape \n e \t"     # aspas duplas com escape sequences
'linha com escape \n e \t'     # aspas simples (idêntico às duplas)
"""texto cru multilinha
   sem escape sequences"""     # três aspas: crua (raw) multilinha
```

Não existe interpolação léxica embutida. A injeção de variáveis em texto é
delegada à função `format`.

### Palavra-chave Lambda

As palavras-chave `lambda` e o caractere unicode `λ` são perfeitamente
intercambiáveis e equivalentes em qualquer contexto.

### Terminador de Statement (`;`)

O ponto e vírgula `;` é um **terminador de statement opcional** no domínio das
Actions. Seu uso é exclusivamente para permitir múltiplos statements na mesma
linha ou para explicitar que uma expressão não é um valor de retorno:

```kata
action processar
    let x := 5; echo!(x)       # dois statements na mesma linha
    let y := + x 1
    y                           # retorno implícito (última expr sem ;)
```

Quando a última expressão de uma Action termina com `;`, a Action retorna
`Unit`. Sem `;`, a última expressão é o retorno implícito. O `;` distingue
"computação local" de "valor que escapa" — esta distinção é o que habilita o
caller's arena (ver §5.2).

O `;` não entra em conflito com seus outros usos em coleções (separador de
dimensões em tensores `{1; 2; 3}`), pois vive em contexto de statement, fora de
delimitadores de coleção.

## 1. Interface de Linha de Comandos (CLI)

O binário `kata` expõe os seguintes comandos para a gestão do ciclo de vida do
código:

* **`lex <arquivo.kata>`**: Executa a análise léxica e imprime a lista de tokens
  com os respetivos *spans* no terminal. Útil para depuração do lexer.
  ```bash
  kata lex examples/test_fizzbuzz.kata
  ```
* **`parse <arquivo.kata>`**: Executa a análise léxica e sintática e imprime a
  AST completa via `Debug` pretty-print. Útil para depuração do parser.
  ```bash
  kata parse examples/test_enum.kata
  ```
* **`build <arquivo.kata>`**: Compila um ficheiro de entrada (*entrypoint*) para
  binário nativo (AOT). Pipeline completo: lex → parse → resolution → inference
  → monomorph → escape → tree shaking → comptime → lowering → CLIF → Cranelift
  AOT → object file → link → executável.
  ```bash
  kata build examples/test_fatorial.kata
  ```
* **`run <arquivo.kata>`**: Compila e executa o código em modo JIT via Cranelift.
  Carrega o prelude, resolve `import` recursivamente, e invoca a função
  `__kata_entry`. Aceita a flag `--emit-ir` para imprimir a CLIF canônica antes
  da execução.
  ```bash
  kata run examples/test_simple.kata
  kata run --emit-ir examples/test_simple.kata
  ```
* **`test <arquivo.kata>`**: Invoca o *Test Runner* nativo. Descobre diretivas
  `@test("descrição")` e `@test{desc: "...", expects: "CompileError"}`, executa
  cada teste em JIT isolado, e reporta contagem pass/fail/error com exit code
  apropriado.
  ```bash
  kata test examples/test_assert.kata
  ```
* **`repl`**: Inicia o REPL interativo com `TypeEnv` persistente e histórico
  persistente (`~/.kata_repl_history`). Suporta comandos especiais `:help`,
  `:type <expr>`, `:env`, `:load <file>`, `:reset`, `:quit`. Multiline para
  `match`, `enum`, `interface`, `implements`, assinaturas de função (`Sig` +
  `lambda`), e `action`. Erros não abortam a sessão (rollback automático).
  ```bash
  kata repl
  ```
  Ver §26 para detalhes.
* **`eval <expressão>`**: Avalia uma expressão via JIT de forma não-interativa,
  retornando o resultado no stdout. Útil para scripts e one-liners. Aceita a
  flag `--emit-ir`.
  ```bash
  kata eval '+ 1 2'
  kata eval --emit-ir '+ 1 2'
  ```

## 2. Pipeline de Compilação

O motor de compilação aplica uma sequência estrita e abortiva (falhas num
estágio impedem o avanço para o seguinte). O pipeline é uma cadeia linear sem
back-edges. Cada crate consome a saída do anterior e não olha para trás.

### 2.1. Decisão Arquitetural: Sem IR Intermediária Própria

Iterações anteriores do compilador tinham uma IR intermediária (`kata-ir`)
entre a TAST e o Cranelift. Esta camada tornou-se uma camada de tradução
redundante que perdia semântica. Kata-Lang **não tem IR intermediária própria** —
o lowering é direto TAST → CLIF (Cranelift IR).

A semântica que o backend precisa é preservada de duas formas:

1. **TAST enriquecida** (§2.5): cada nó carrega `ty`, `escape`, `capture`,
   `tail_pos`, `mono_instance`, `effect`. O lowering lê estes campos diretamente.
2. **MetadataTable sidecar** (§2.6): snapshot read-only do estado semântico
   pós-lowering, indexado por `Inst`/`Block` do Cranelift. O ARC pass consulta
   esta tabela.

O Cranelift faz o que sabe fazer bem (register allocation, instruction selection,
const folding, DCE, inlining, TCO). O kata-optimizer faz o que precisa de
semântica (TRMA, StreamFusion, ARC pass). Não há duplicação de análise.

### 2.2. Visão Geral das Camadas

A TAST semântica é o ponto de separação: tudo antes dela é construção de tipos
e validação, tudo depois é lowering mecânico para CLIF (Cranelift IR). Não
existe IR intermediária própria — o lowering é direto TAST → CLIF, preservando
semântica via uma MetadataTable sidecar.

- **Fundação (`kata-core`):** Não é uma fase do pipeline — é a fundação
  transversal importada por todas as fases. Define `Ty` (tipo canônico),
  `TypeEnv` (árvore de escopos), `FfiSymbol` (enum tipado de símbolos FFI com
  metadados), `TypeShape` (projeção runtime para reflexão estrutural), e
  `type_id` (identificador u32 para a type table do runtime). É o contrato
  compartilhado entre camadas.

- **Frontend (`kata-lexer` + `kata-parser` + `kata-ast`):** Converte texto em
  AST plana. O lexer é indent-sensitive (emite `INDENT`/`DEDENT` sintéticos). O
  parser é recursive-descent, prefix-only (sem Pratt). O AST é um crate separado
  (`kata-ast`) de dados puros, sem lógica.

- **Módulos (`kata-module-loader`):** Carregamento de módulos do filesystem com
  cache e detecção de ciclos. `load_prelude` injeta a stdlib (`core`)
  automaticamente, produzindo `TypeEnv` + `DispatchTable` + `InterfaceRegistry`
  iniciais. O typeck não sabe da existência de arquivos.

- **Resolution (`kata-resolution`):** Pass 0 + Pass 1. Popula `TypeEnv`,
  resolve imports, coleta assinaturas, expande smart constructors. Produz o
  `ResolvedModule` (imutável).

- **Inference (`kata-inference`):** Pass 2. Type-check dos corpos, inferência,
  dispatch por dominância, currying, ascription, pattern matching, guards,
  análise CSP (type-level). TRMA roda dentro de inference (reescrita com
  acumulador, muta TAST). Produz o `TypedModule` (TAST).

- **Monomorphization (`kata-monomorph`):** Especializa call sites genéricos,
  resolve impls concretos. Produz o `MonoModule` (TAST com tipos concretos).

- **Escape Analysis (`kata-escape`):** 4 passes sobre a TAST, marca
  `CaptureStorage` Stack/Heap. Produz o `AnnotatedModule` (TAST + anotações de
  escape).

- **Tree Shaking (`kata-tree-shaking`):** Dead code elimination. Worklist
  iterativa a partir das Actions, mark & filter. Produz o `ReachableModule`
  (TAST podado).

- **Comptime (`kata-comptime`):** Avalia expressões `@comptime` via
  JIT-and-execute (compila a expressão usando o pipeline normal e executa no
  `kata-rt` real). Substitui por `HeapSnapshot` na TAST. Coleta closure defs e
  snapshots para carga no runtime.

  **Por que JIT-and-execute (não interpretador de TAST):** Um interpretador de
  TAST foi considerado e rejeitado (Fio 14). Razões:

  - **Sem reimplementação:** o runtime já sabe avaliar tudo — `match`, guards,
    lambda, recursão, FFI, listas, structs. Zero código duplicado.
  - **Sem teto:** cobre tudo que o codegen compila. Um interpretador teria
    cobertura limitada aos tipos que `ConstValue` representa.
  - **Consistência semântica:** comptime e runtime usam exatamente o mesmo
    código. Não há risco de divergência.
  - **Heap snapshot:** tipos complexos (listas, structs, enums com payload) são
    avaliados via JIT; o resultado é um ponteiro para dados nativos no heap. O
    snapshot é o estado inicial do heap runtime, carregado em load-time com
    fix-up table para ponteiros absolutos.

- **Lowering + Emit (`kata-codegen`):** Converte o TAST diretamente em CLIF
  (Cranelift IR). Block arguments nativos (Cranelift 0.133) — sem stack slots.
  Produz `cranelift::Function` + `MetadataTable` sidecar (read-only).

- **Optimizer (`kata-optimizer`):** Passes no TAST (TRMA, StreamFusion de
  map/filter/fold). Passes pós-lowering (ARC pass via metadata). Constant
  folding, DCE, inlining e TCO são delegados ao Cranelift, que os executa
  nativamente.

- **Runtime (`kata-rt`):** Biblioteca nativa isolada, linkada via symbol map.
  Scheduler cooperativo single-threaded (struct explícita, não TLS global),
  channels com Mutex/Condvar, arena per-fiber, `Arc<T>` nativo para heap.
  `spawn!` multiprocess via fork + IPC. Desconhece as regras internas da
  linguagem — comunicando-se apenas via C-ABI.

- **Diagnostics (`kata-diagnostics`):** Catálogo central de erros estruturados
  com `miette` para spans coloridos. Organizado em 3 submódulos (frontend,
  middleend, backend) sem códigos numéricos — códigos namespaced por domínio
  (ex: `type.mismatch`, `parse.unexpected_token`). Usado por todas as fases.

- **Driver (`kata-driver`):** Crate do binário `kata`. Orquestra o pipeline:
  lex → parse → module load → resolution → inference → monomorph → escape →
  tree shaking → comptime → lowering → optimize → emit → JIT/AOT → execução.

### 2.2. Diagrama do Pipeline

```
                        KATA5 — PIPELINE DE COMPILAÇÃO
                        ═══════════════════════════════

  source string
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-lexer                                         │
│  char-by-char scanner + IndentTracker              │
│  emite INDENT/DEDENT/StmtSep sintéticos             │
│  saída: Vec<(Token, Span)>                          │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-parser                                        │
│  recursive-descent, prefix-only (sem Pratt)         │
│  saída: Spanned<Expr>  (AST flat, crate kata-ast)   │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-module-loader                                 │
│  load + cache + cycle detection                     │
│  load_prelude → TypeEnv + DispatchTable +           │
│                  InterfaceRegistry                  │
│  saída: AST + ModuleGraph                           │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─ FRONTEND ──────────────────────────────────────────┐
│  kata-resolution                                    │
│  Pass 0: popula TypeEnv (Data→Struct, Enum→Sum,     │
│          Interface, Implements)                     │
│  Pass 1: coleta assinaturas + smart constructors    │
│  saída: ResolvedModule (imutável)                   │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─ MIDDLEEND ─────────────────────────────────────────┐
│  kata-inference                                     │
│  Pass 2: type-check dos corpos                      │
│          (inferência, dispatch, currying,           │
│           ascription, patterns, guards, csp)        │
│  + TRMA (reescrita com acumulador, muta TAST)       │
│  saída: TypedModule (TAST)                          │
│                                                     │
│  TAST enriquecida:                                  │
│    ty: Ty, escape: EscapeKind,                      │
│    capture: Vec<CaptureInfo>, tail_pos: bool,       │
│    mono_instance: u64, effect: Effect               │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-monomorph                                     │
│  Especializa call sites genéricos                   │
│  Resolve impls concretos                            │
│  saída: MonoModule                                  │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-escape                                        │
│  4 passes: marca CaptureStorage Stack/Heap          │
│  Pass 0: closures em retorno de funções puras       │
│  Pass 1: inspeção sintática (Send/Fork/ListLit/...) │
│  Pass 2: propagação de aliases                      │
│  Pass 3: promoção Stack → Heap                      │
│  saída: AnnotatedModule                             │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-tree-shaking                                  │
│  Worklist from actions, mark & filter               │
│  Elimina @test antes da análise de reachability     │
│  saída: ReachableModule                             │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  ══ TAST se torna READ-ONLY aqui ══                 │
│  kata-comptime                                      │
│  JIT-and-execute: compila via pipeline normal,      │
│  executa no kata-rt real, captura HeapSnapshot      │
│  Substitui @comptime por snapshot_id na TAST        │
│  Coleta closure_defs + snapshots                    │
│  saída: ReachableModule + ComptimeArtifacts         │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─ BACKEND ───────────────────────────────────────────┐
│  kata-codegen (lowering + emit consolidados)        │
│                                                     │
│  Lowering TAST → CLIF (Cranelift 0.133)             │
│  Block arguments nativos (sem stack slots)          │
│  + MetadataTable sidecar (read-only):               │
│    inst_origins, block_origins, value_types,        │
│    closure_info, escape_flags                       │
│                                                     │
│  saída: cranelift::Function + MetadataTable         │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  kata-optimizer                                     │
│                                                     │
│  Passes no TAST (já executados em inference):       │
│    ✓ TRMA — mantido (precisa semântica)             │
│    ✓ StreamFusion — map/filter/fold fusion          │
│                                                     │
│  Passes pós-lowering (consultam metadata):          │
│    ✓ ARC pass — incref/decref via Arc<T> nativo     │
│                                                     │
│  Delegados ao Cranelift 0.133:                      │
│    ✗ TCO, const fold, DCE, inlining — nativos       │
└─────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────┐
│  Cranelift JIT (run/eval/repl)  ou  AOT (build)     │
│  define_function × N, finalize_definitions          │
│  carrega snapshots no bump alloc global             │
│  extrai __kata_entry, transmute → extern "C"        │
│  call()                                             │
└─────────────────────────────────────────────────────┘
      │
      ▼
   resultado (i64)
```

### 2.3. Runtime (kata-rt)

Biblioteca nativa isolada, linkada no JIT/AOT via symbol map. O compilador
conhece apenas o enum `FfiSymbol` (tipado, com metadados) e as 3 strings de
mapeamento (`"i64"`, `"f64"`, `"kata_rt_string"`) — toda a implementação vive
em `kata-rt`, desacoplada do compilador.

- **Aritmética Inteira:** `kata_rt_iadd`, `kata_rt_isub`, `kata_rt_imul`,
  `kata_rt_idiv`
- **BigInt (precisão arbitrária, SMI tagging):** `kata_rt_bi_add`, `kata_rt_bi_sub`,
  `kata_rt_bi_mul`, `kata_rt_bi_div`, `kata_rt_bi_eq`, ..., `kata_rt_tag_int`
- **Bitwise (BigInt):** `kata_rt_and`, `kata_rt_or`, `kata_rt_not`
- **Comparação Inteira:** `kata_rt_icmp_eq`, `kata_rt_icmp_lt`, `kata_rt_icmp_gt`, ...
- **Aritmética Float:** `kata_rt_fadd`, `kata_rt_fsub`, `kata_rt_fmul`, `kata_rt_fdiv`
- **Comparação Float:** `kata_rt_fcmp_eq`, `kata_rt_fcmp_lt`, ...
- **Rational:** `kata_rt_rat_add`, ..., `kata_rt_rat_literal`, `kata_rt_rat_show`,
  `kata_rt_rat_to_float`, `kata_rt_rat_from_float`, `kata_rt_int_to_rational`
- **I/O:** `kata_rt_print`
- **Textos:** `kata_rt_string_concat`, `kata_rt_string_len`, `kata_rt_text_literal`,
  `kata_rt_int_to_text`, `kata_rt_bool_to_text`, `kata_rt_text_replace_first`
- **Arena:** `kata_rt_arena_create`, `kata_rt_arena_alloc`, `kata_rt_arena_destroy`
- **Coleções — Listas:** `kata_rt_list_nil`, `kata_rt_list_cons`, `kata_rt_list_is_empty`,
  `kata_rt_list_head`, `kata_rt_list_tail`
- **Coleções — Arrays:** `kata_rt_array_alloc`, `kata_rt_array_len`, `kata_rt_array_get`,
  `kata_rt_array_set`
- **Sum:** `kata_rt_store_sum_result`, `kata_rt_tag_int`
- **CSP:** `kata_rt_channel_create`, `kata_rt_channel_send`,
  `kata_rt_channel_recv`, `kata_rt_queue_create`, `kata_rt_broadcast_create`,
  `kata_rt_select` (com timeout), `kata_rt_fork` (scheduler cooperativo)
- **ARC (Arc<T> nativo):** `kata_rt_alloc_arc`, `kata_rt_incref`, `kata_rt_decref`
- **Type Table (reflexão estrutural):** `kata_rt_register_type`,
  `kata_rt_register_type_arena`, `kata_rt_typeof`, `kata_rt_repr_to_text`
- **Pretty Printing:** `kata_rt::pretty_print(ptr, type_id, max_depth)`
- **Logging (`@log`):** `kata_rt_log_publish`, `kata_rt_log_recv`, `kata_rt_log_config`
- **Comptime/Snapshots:** `kata_rt_struct_to_bump`, `kata_rt_load_snapshot`,
  `kata_rt_get_snapshot_root`, `kata_rt_register_fn`
- **Fuel:** `kata_rt_fuel_decrement`, `kata_rt_fuel_exhausted`

### 2.4. Fundação Transversal (kata-core)

O `kata-core` não é uma fase do pipeline — não consome nem produz dados do
fluxo. É importado por todas as fases como dependência de biblioteca,
fornecendo os contratos compartilhados entre camadas:

- **`Ty` / `PrimTy` / `VariantType`** — Tipo canônico: `Prim(Int|Float|Text|Rational)`,
  `Unit`, `Struct`, `Sum`, `Function`, `Tuple`, `Generic`, `Interface`, `List`,
  `Array`, `Channel`, `Queue`, `Broadcast`, `User`, `Refined`, `Range`, `InferVar`.
  O typeck produz `Ty` em cada `TypedExpr.ty`. O lowering mapeia `Ty` direto para
  ABI do Cranelift (Int→I64, Float→F64, Text/Struct/Sum/...→Ptr).

- **`TypeEnv`** — Árvore de escopos (parent + bindings). Populada no resolution
  e inference para resolver nomes. Não sobrevive além do typeck — o TAST já
  carrega os tipos resolvidos em cada nó.

- **`FfiSymbol`** — Enum tipado de símbolos FFI. Cada variante carrega o nome do
  símbolo (`symbol_name()`) e metadados (`return_type()`, etc.). Substitui o
  catálogo de strings soltas — se você errar o símbolo, é erro de
  compilação do compilador, não bug silencioso em runtime.

- **`TypeShape`** — Projeção runtime de `Ty` para reflexão estrutural. Descarta
  InferVar/Generic/Interface (mapeados para Unit/User graceful). O codegen emite
  `register_type(ptr, type_id)` após cada `alloc_arc` e
  `register_type_arena(ptr, type_id)` após cada `arena_alloc`, permitindo que
  `typeof` e `pretty_print` funcionem em tempo de execução.

- **`type_id`** — Identificador `u32` atribuído em compile-time para cada `Ty`
  distinto no módulo. Serve de chave para a type table estática no runtime.

### 2.5. O Princípio "Sem Builtins"

A decisão arquitetural mais original do projeto: **o compilador não tem
builtins**. Tudo — aritmética, comparação, strings, coleções, I/O — é definido
na stdlib em código Kata via `@ffi`. O compilador conhece apenas:

1. O enum `FfiSymbol` (catálogo tipado de símbolos FFI)
2. As 3 strings de mapeamento de representação (`"i64"`, `"f64"`,
   `"kata_rt_string"`)
3. A diretiva `@builtin` para interceptação de funções específicas (map/filter/fold)

Nenhum operador (`+`, `-`, `=`, `<`) tem tratamento especial no parser, typeck,
ou codegen. São identificadores comuns definidos na stdlib com `@ffi` apontando
para funções em `kata-rt`. A clareza conceitual compensa o boilerplate de stdlib.

Este princípio foi validado de forma não-trivial: SMI tagging para
Int de precisão arbitrária foi implementado **inteiramente no `kata-rt`**, sem
mudanças no typeck, IR, optimizer, ou emit. O compilador continua vendo `i64` —
o runtime decide representação. Isso prova que o modelo "sem builtins" escala
até para otimizações de runtime que poderiam parecer exigir conhecimento do
compilador.

**Princípio derivado: transparência runtime.** Otimizações de runtime que não
exigem conhecimento do compilador deveriam viver inteiramente no runtime. O
compilador vê o tipo canônico (`Int` → `i64`); o runtime decide representação
(SMI inline ou heap BigInt). Não poluir o typeck/codegen com detalhes de
representação de runtime.

### 2.6. TAST Semântica (Enriquecida)

A TAST do Kata-Lang carrega **toda a semântica que o backend precisa**, para que o
lowering TAST → CLIF seja direto (sem reconstrução de informação). Cada nó da
TAST possui:

| Campo | Descrição |
|---|---|
| `ty: Ty` | Tipo canônico |
| `escape: EscapeKind` | NãoEscapa / EscapaParaHeap / EscapaParaClosure |
| `capture: Vec<CaptureInfo>` | O que esta lambda captura e como |
| `tail_pos: bool` | Esta expressão está em posição de cauda? |
| `mono_instance: u64` | Qual versão monomorfizada esta chamada resolve |

Com isso, o lowering é direto: cada nó da TAST já carrega tudo que o CLIF
precisa. O optimizer consulta a MetadataTable (snapshot pós-lowering) para
decisões informadas por tipo sem precisar refazer análise.

### 2.7. MetadataTable Sidecar

O lowering produz CLIF + uma tabela de metadata paralela, indexada por `Inst` e
`Block` do Cranelift:

```rust
struct MetadataTable {
    inst_origins:   HashMap<cranelift::Inst, TastNodeId>,
    block_origins:  HashMap<cranelift::Block, TastNodeId>,
    value_types:    HashMap<cranelift::Value, Ty>,
    closure_info:   HashMap<cranelift::FuncId, ClosureMeta>,
    escape_flags:   HashMap<cranelift::Value, EscapeKind>,
}
```

Read-only após lowering. Consultada pelo ARC pass. Não modificada por ninguém.
O Cranelift não sabe que existe — é um snapshot do estado semântico pós-lowering.

## 3. Sistema de Módulos e Resolução

A gestão de dependências adota uma abordagem mista para equilibrar segurança e
conveniência.

### 3.1. Visibilidade e Exportação

Tudo definido dentro de um módulo é visível dentro do mesmo módulo. O que é
importado torna-se visível dentro do módulo. **Apenas o que é exportado pode ser
importado por outros módulos.**

A exportação é explícita, usando a palavra-chave `export` seguida dos itens
separados por espaço:

```kata
export nome_funcao TipoX + -        # itens exportados separados por espaço
export tipos.(Int Float Boolean)    # reexportação: MOD.(itens) — parênteses só para o grupo
```

Vírgulas são opcionais entre itens. A forma `MOD.(itens)` exporta itens de um
submódulo como se fossem do módulo atual (reexportação).

### 3.2. Importação

```kata
import utilidades.matematica                 # import de módulo inteiro
import utilidades.matematica as mat           # com alias
import utilidades.(matematica TipoX IFACE)    # import seletivo de itens específicos
```

### 3.3. Injeção Automática do Prelude (`core`)

O prelude (`core`) é carregado automaticamente em todos os ficheiros executados
como *entrypoint*, disponibilizando os seus itens exported globalmente sem
necessidade de prefixo (ex: `echo!` direto, não `core.echo!`). Módulos
secundários não importam o `core` magicamente — cada módulo resolve seus tipos
independentemente. O carregamento do prelude é responsabilidade do
`kata-module-loader`, não do typeck.

### 3.4. Resolução de Caminhos (Path Resolution)

A importação de um módulo como `utilidades.matematica` é resolvida substituindo
os pontos por separadores de sistema. O `ModuleLoader` pesquisa em duas vertentes:
1.  **Ficheiro Direto**: Procura a existência exata de `utilidades/matematica.kata`.
2.  **Agregador de Diretório**: Caso falhe, procura `utilidades/matematica/mod.kata`.

### 3.5. Prevenção de Ciclos

O compilador gere um *cache* partilhado das TASTs e dos Ambientes de Tipos
(`TypeEnv`). Quando um módulo é solicitado, a infraestrutura verifica o *cache*
primariamente; se já estiver registado, devolve a referência imediatamente,
impedindo avaliações redundantes e travamentos por dependências circulares.

## 4. Sistema de Tipos e Despacho Múltiplo

O sistema de tipos da Kata-Lang opera sob uma filosofia de **Verificação
Antecipada (Early Checking)**. Ao contrário de templates em C++ ou tipagem pato
(duck typing), funções genéricas são provadas matematicamente na sua definição
antes de serem instanciadas, utilizando contratos (Interfaces).

### 4.1. Interfaces e Contratos (Super-Traits)

A linguagem não possui classes. O polimorfismo *ad-hoc* é atingido via *Multiple
Dispatch*. As interfaces (escritas em `ALL_CAPS`) definem assinaturas obrigatórias.
O motor do compilador (no *middle-end*) pontua os tipos concretos baseando-se
nestas restrições para selecionar a implementação correta no momento do despacho.

**Declaração de Interface:**

```kata
interface NUM implements ORD EQ
    + :: NUM NUM => NUM
    - :: NUM NUM => NUM
    * :: NUM NUM => NUM
    abs :: NUM => NUM
    div :: NUM NUM => Result::(NUM, Err)
```

A sintaxe é `interface NOME` seguido opcionalmente de `implements
SUPERINTERFACE...` e um bloco indentado contendo as assinaturas obrigatórias
(definições vazias de função, apenas com `::` e `=>`). O nome da interface pode
ser usado como tipo nos parâmetros e retorno das suas próprias assinaturas,
indicando "qualquer tipo que implemente esta interface".

A herança de super-traits propaga obrigações: implementar `NUM` exige
implementar todas as funções de `NUM`, `ORD` e `EQ` de uma vez. Quando `Int`
implementa `NUM`, as funções de `ORD` e `EQ` são fornecidas como parte do mesmo
bloco `implements` — o tipo automaticamente satisfaz as três interfaces.

**Interoperabilidade entre tipos da mesma interface**: Tipos que aderem
diretamente a uma interface são interoperáveis. Cada função da interface (ex:
`=`, `+`, `<`) opera sobre pares de `NUM`, não apenas sobre o próprio tipo. Cada
tipo define cláusulas para os tipos já existentes mais uma cláusula genérica
(`T NUM`) como fallback para tipos futuros. A responsabilidade de integração
recai sobre o tipo novo, não sobre os estabelecidos.

#### 4.1.1. Interfaces Parametrizadas (Genéricas)

Interfaces podem ter **type params** — parâmetros de tipo declarados entre
parênteses após o nome. A interface `ITERABLE(A)` é o exemplo canônico na stdlib:

```kata
interface ITERABLE(A)
    next :: Self => Optional(A)
```

O type param `A` representa o tipo do elemento iterado. Ao declarar `impl`, o
tipo concreto vincula seus próprios type params ao pattern da interface:

```kata
List(A) implements ITERABLE(A)
    next :: List(A) => Optional(A)

Array(A) implements ITERABLE(A)
    next :: Array(A) => Optional(A)
```

O motor do typeck registra cada `impl` como um `ImplEntry` contendo:
- **`type_pattern`** — o tipo concreto com type params resolvidos (ex: `List(A)`)
- **`iface_params`** — os parâmetros da interface instanciados (ex: `A`)
- **`type_params`** — os nomes dos type params do tipo (ex: `A`)

Quando o dispatch encontra uma call sobre `ITERABLE(A)`, ele unifica o
`type_pattern` do `ImplEntry` com o tipo concreto do receptor para instanciar o
impl correto.

### 4.2. Construtores Universais de Tipo (Smart Constructors)

A Kata-Lang adota um princípio unificador: **todo nome de tipo (`CamelCase`) é
invocável como função pura**. A sintaxe `Nome :: ArgType => Self` define o
construtor — recebe o tipo base e retorna o valor tipado. Isso proporciona uma
interface uniforme para criar valores de `data` (refined e struct), `alias` e
`enum` sob uma única regra, consistente com a notação prefixada da linguagem.

#### 4.2.1. Quando o typeck sintetiza

O compilador sintetiza construtores automaticamente quando um tipo é
declarado. A síntese acontece em **resolution** (coleta de assinaturas), antes
do type-check dos corpos. O construtor existe como uma função no `TypeEnv`
antes de qualquer corpo ser verificado.

**Regra de síntese:** todo tipo declarado gera um construtor. A assinatura e o
corpo dependem da natureza do tipo:

| Declaração | Construtor | Assinatura | Retorno | Corpo |
|---|---|---|---|---|
| `data Pessoa (nome::Text, idade::Int)` | `Pessoa` | `Pessoa :: Text Int => Pessoa` | Direto | `StructAlloc` + `FieldStore` por campo |
| `data (Int, > _ 0) as PositiveInt` | `PositiveInt` | `PositiveInt :: Int => Result::(PositiveInt, Error)` | Falível | Guard chain com predicados → `Ok`/`Err` |
| `enum IMC` com predicados | `IMC` | `IMC :: Float => IMC` | Direto | Guard chain por variante, primeira que satisfaz vence, variante default é fallback |
| `enum Result { Ok(T), Err(E) }` | `Ok`, `Err` | `Ok :: T => Result::(T, E)`, `Err :: E => Result::(T, E)` | Direto | Variante como função (não é smart constructor — é função de primeira classe) |
| `enum Boolean { True, False }` | `True`, `False` | `True :: => Boolean`, `False :: => Boolean` | Direto | Variante unitária como constante |
| `alias Float as Altura` | `Altura` | `Altura :: Float => Altura` | Direto | Identity: retorna o parâmetro |
| `alias PositiveInt as PNZ` | `PNZ` | `PNZ :: Int => Result::(PNZ, Error)` | Falível | Delega ao construtor do target |

**A linha que distingue:** se há **predicados**, o construtor é sintetizado com
guard chain. Se não há predicados, é variante como função/constante ou struct
alloc. O typeck não sintetiza guard chain para `Ok(T)` — `Ok` é uma função que
empacota o valor na variante, sem validação.

#### 4.2.2. Tipos Refinados (`data` com predicados)

A sintaxe `data (Int, > _ 0) as PositiveInt` cria um tipo nominal cuja base
estrutural é `Int`, mas que carrega um predicado matemático. O compilador
sintetiza automaticamente:

```kata
PositiveInt :: Int => Result::(PositiveInt, Error)
lambda v:
    > v 0: Ok(v)
    otherwise: Err("predicado > _ 0 falhou em PositiveInt v")
```

* **Retorno Falível:** O construtor devolve `Result::(T, Error)`, forçando o
  programador a lidar com a falha lógica — o "atrito sadio".
* **Corpo com Guards:** O predicado é avaliado via `Guard` encadeado — se passar,
  retorna `Ok(v)`; se falha, retorna `Err(código)`.
* **Múltiplos predicados:** Se houver mais de um predicado, todos devem ser
  satisfeitos (AND lógico). Cada predicado gera uma cláusula no `Guard`. O
  primeiro que falha aborta a cadeia e retorna `Err`.
Exemplo de uso:

```kata
action validar
    let x := PositiveInt 42 ?            # ? desempacota Result em Actions
    echo!(x)
validar!()
```

Em funções puras, `?` não existe — use `match` explícito:

```kata
extrai :: Result::(PositiveInt, Error) => Int
lambda r:
    match r
        Result::Ok(v): v
        Result::Err(_): 0
        otherwise: 0
```

#### 4.2.3. Tipos Produto (`data` struct)

```kata
data Pessoa (nome::Text idade::Int)
```

Construtor sintetizado infalível — todos os campos são aceitos sem validação.
Cada argumento mapeia posicionalmente para um campo. O corpo sintetizado é
`StructAlloc` seguido de `FieldStore` por campo, na ordem inversa (último campo
primeiro, primeiro campo por último — para que o `let` chain termine com o
primeiro campo no topo).

O construtor existe no `TypeEnv` antes do type-check dos corpos. Quando o
usuário escreve `Pessoa "João" 30`, o typeck despacha para o construtor
sintetizado, que aloca a struct e preenche os campos.

**Construtores sintetizados usam tipos concretos dos campos, não interfaces.**
`data Complex (re::Float, im::Float)` gera `Complex :: Float Float => Complex`,
não `Complex :: NUM NUM => Complex`. A assinatura é determinada pelos tipos
declarados dos campos — o construtor precisa saber exatamente qual tipo
esperar para alocar e preencher a struct.

Isto previne auto-referência: se o construtor aceitasse `NUM`, `Complex` (que
implementa `NUM`) seria aceito como argumento de `Complex`, criando a
possibilidade de `Complex c1 c2` despachar para o próprio construtor de
`Complex`. Com tipos concretos, `Complex c1 c2` é type error — `Complex` não é
`Float`.

**Overloads manuais para aceitar outros tipos.** O construtor sintetizado é
uma overload como qualquer outra. O usuário pode adicionar overloads para
aceitar tipos diferentes:

```kata
data Complex (re::Float, im::Float)

# Construtor sintetizado (infalível) — Float Float
# Complex :: Float Float => Complex  (sintetizado pelo typeck)

# Overload manual — Int Int
Complex :: Int Int => Complex
lambda re im: Complex (from_int re) (from_int im)
```

Multiple dispatch seleciona por tipo de argumento: `Complex 3 4` → `[Int, Int]`
→ overload manual. `Complex 3.0 4.0` → `[Float, Float]` → sintetizado. Isto é
consistente com o princípio de que não há coerção implícita — aceitar `Int`
onde se espera `Float` é uma decisão explícita do usuário, não automática.

**O construtor sintetizado NÃO é sobrescrito pela overload manual.** Ambas
coexistem no `DispatchTable`, despachadas por tipo de argumento. O typeck
seleciona por dominância — `[Float, Float]` matches sintetizado, `[Int, Int]`
matches manual.

#### 4.2.4. Variantes de Enum (funções de primeira classe)

Variantes de enum **sem predicados** não são smart constructors sintetizados —
são funções de primeira classe que o typeck registra como overloads:

- Variante com payload `Ok(T)` tem tipo `T -> Result::(T, E)`. O codegen emite
  `EnumVariant` com payload.
- Variante unitária `True` tem tipo `-> Boolean` (constante). O codegen emite
  `EnumVariant` sem payload.

A distinção com smart constructor: `Ok(T)` apenas empacota o valor na variante.
Não há validação, não há guard chain, não há `Result` no retorno do construtor
— o retorno é o `EnumType` diretamente. A variante **é** o valor.

#### 4.2.5. Enum Predicado (smart constructor falível com despacho)

```kata
enum IMC
    Magreza(< _ 18.5)
    Normal(<= _ 25.0)
    Sobrepeso(<= _ 30.0)
    Obesidade           # variante default (sem predicado)
```

O construtor sintetizado despacha para a variante cujo predicado satisfaz:

```kata
IMC :: Float => IMC
lambda x:
    < x 18.5: Magreza(x)
    <= x 25.0: Normal(x)
    <= x 30.0: Sobrepeso(x)
    otherwise: Obesidade           # fallback sem predicado
```

* **Retorno direto (não `Result`):** O construtor de enum predicado sempre
  produz uma variante — a variante default garante cobertura total. Não há
  falha, apenas despacho.
* **Corpo com Guards:** Cada variante predicada é um Guard. O primeiro
  predicado satisfeito captura o valor na variante correspondente. A variante
  default (última, sem predicado) é o `otherwise`.
* **Regras:** Se uma variante tem predicado, todas (exceto a última) devem ter.
  A última é o fallback sem predicado.
* **Reusa maquinaria de refined:** O padrão de guard chain é o mesmo de tipos
  refinados. A diferença é o retorno: refined retorna `Result`, enum predicado
  retorna `Sum` direto.

#### 4.2.6. Aliases (Herança de Construtor)

O alias herda a assinatura e semântica do construtor do tipo base, trocando
apenas o invólucro nominal:

| Declaração | Tipo base | Construtor sintetizado | Corpo |
|-----------|-----------|------------------------|-------|
| `alias Float as Altura` | `Float` (primitivo) | `Altura :: Float => Altura` | Identity: retorna `__x` tipado como `Altura` |
| `alias PositiveInt as PositiveNonZero` | `PositiveInt` (refined) | `PositiveNonZero :: Int => Result::(PositiveNonZero, Error)` | Delega: `PositiveInt(__x)` |
| `alias Matrix as MatrizLocal` | `Matrix` (struct) | `MatrizLocal :: Text Int => MatrizLocal` | Delega: `Matrix(__x_1, __x_2)` |

O alias preserva exatamente a mesma semântica de falha do tipo base:
- Se o target é infalível (primitivo, struct), o alias é infalível (identity ou
  delega).
- Se o target é falível (refined), o alias é falível (delega ao construtor do
  target, que retorna `Result`).

O principal motivo de existência do `alias` é resolver a **Regra de Coerência
(Orphan Rule)** — permite implementar uma interface externa num tipo externo
encapsulando-o localmente.

#### 4.2.7. Ascription — três modos semânticos

`::` é um operador (reconhecido pelo parser desde Fio 1). Tem três modos
que se distinguem pelo que fazem em runtime:

**1. Rebaixamento de literal** — o texto bruto do literal é reinterpretado
no tipo alvo desde o início, sem conversão em runtime:

```kata
42::Float        # "42" nasce como f64, não Int convertido
3.14::Rational   # "3.14" nasce como Rational, nunca passa por f64
1::Rational      # "1" nasce como Rational
```

Rebaixamento só se aplica a literais. `x::Float` onde `x` é variável Int
não rebaixa — é `TypeMismatch` (use `from_int x`). O codegen inspeciona
`(literal_kind, target_ty)` para decidir o símbolo FFI: `IntLit→Float` =
`f64 const`, `IntLit→Rational` = `kata_rt_rat_literal`,
`FloatLit→Rational` = `kata_rt_rat_literal`.

**2. Confirmação de tipo** — verifica que a expressão já tem o tipo alvo.
No-op em runtime:

```kata
42::Int          # já é Int, ascription é anotação defensiva
x::Int           # se x é Int, OK; senão TypeMismatch
```

**3. Ascription-construção** (Fio 5+) — promove uma tupla anónima a um
tipo nominal, alocando com `type_id` e copiando campos:

```kata
("João" 30)::Pessoa   # tupla (Text, Int) → Pessoa com type_id
```

A tupla carrega a mesma forma que o struct. O `::Pessoa` valida que os
**tipos** dos elementos batem com os campos (verificação de shape) e
anexa a identidade nominal. A identidade é extrínseca — a tupla é
anónima, a ascription é o que a promove.

#### 4.2.8. Ascription vs Construtor — diferenças

Ascription-construção e construtor sintetizado ambos produzem valores
de tipos user-defined, mas diferem em quatro pontos:

| | Construtor | Ascription-construção |
|---|---|---|
| Identidade | Nominal (intrínseca) | Estrutural → nominal (extrínseca) |
| First-class | Sim (valor no DispatchTable) | Não (sintaxe) |
| Validação de shape | Tipos dos args no dispatch | Forma da tupla vs campos do struct |
| Refinamento | Injeta check (guard chain no corpo) | Avalia predicado local no typeck |

**Identidade:** `Pessoa "João" 30` liga argumentos a campos **por nome
na assinatura** (nominal). `("João" 30)::Pessoa` liga elementos a campos
**por posição** (estrutural). Se alguém reordenar os campos na
declaração, o construtor continua correcto; a ascription parte.

**First-class:** `Pessoa` é um valor — uma função no `DispatchTable`,
passável como argumento, componível. `::Pessoa` é sintaxe inline, não
um valor.

**Validação de shape:** o construtor valida os tipos dos argumentos
na fronteira do dispatch. A ascription valida a forma da tupla contra
os campos do struct. Ambos rejeitam shape incorreto, mas em fronteiras
diferentes.

**Refinamento:** a ascription é onde o predicado do tipo refined é
avaliado — é o ato de atribuir tipo, e o refinamento é parte do tipo.
O construtor tem que importar o check de fora (guard chain no corpo
sintetizado). A ascription valida em compile-time: predicados triviais
(`> _ 0`) são avaliados localmente pelo typeck (avaliação constante,
sem JIT); predicados complexos (`is_prime _`) são delegados ao comptime
pass, que JIT-executa a função predicado e verifica o resultado.
O construtor valida em runtime.

**Falha:** ambos podem falhar com `TypeMismatch`. Ascription additionally
falha com **refinamento não atendido** — o predicado é avaliado em
typeck e rejeita o programa antes do codegen. O construtor falível
devolve `Result::(T, Error)`, forçando o programador a lidar com a
falha via `|` (funções puras) ou `?` (Actions).

```kata
let x := 5::PositiveInt            # ascription: predicado avaliado em typeck
let erro := (-5)::PositiveInt      # type error: predicado falhou em compile-time
let y := PositiveInt 25 ?          # em Actions, ? desempacota Result
let z := match (PositiveInt 42)    # em funções puras, match explícito
    Result::Ok(v): v
    Result::Err(_): 0
    otherwise: 0
```

**Ascription em expressão não-literal (`f(x)::PositiveInt`):** type
error. A ascription exige prova compile-time, não fé. Use o construtor.

**4. Downcast estrutural** (post-refines) — rebaixa um tipo refined ou alias
ao seu tipo base, sem custo em runtime. Os bits são idênticos — refined e
alias são apenas type tags sobre a base:

```kata
data (Int, > _ 0) as PositiveInt
alias Float as Altura

let a := 5::PositiveInt
let n := a::Int              # PositiveInt → Int (downcast, no-op em runtime)

let h := 1.8::Altura
let f := h::Float            # Altura → Float (downcast, no-op em runtime)
```

O typeck verifica que `target_ty` é o tipo base (seguido via `alias_of` no
`StructRegistry`). O codegen emite `bitcast` quando o Cranelift type difere
(ex: `I64→F64` para refined sobre `Float`); para `I64→I64` (refined sobre
`Int`), é literalmente um no-op. Não há validação de predicado — o downcast
preserva o valor bruto, descartando a etiqueta nominal.

O downcast é a válvula de escape para combinar refineds distintos ou para
passar um refined onde a base é esperada, sem recorrer ao construtor falível.
É complementar ao `refines` (§4.4): `refines` habilita interoperabilidade
automática via fallback no dispatch; downcast é a forma explícita.

#### 4.2.9. `repr` como implementação automática de SHOW

Separado dos smart constructors, o typeck sintetiza automaticamente a função
`repr` para tipos `data` com campos. Esta função é a implementação automática
de `SHOW`:

```kata
data Pessoa (nome::Text idade::Int)
# typeck sintetiza:
repr :: Pessoa => Text
lambda x:
    + "Pessoa(" (+ (+ (repr x.nome) ", ") (+ (repr x.idade) ")"))
```

A síntese de `repr` é por tipo de campo:
- `Text`: identity (pass-through)
- `Int`: `kata_rt_int_to_text` via FFI
- `Boolean`: `kata_rt_bool_to_text` via FFI
- `Struct` aninhado: `repr` recursivo (chamada a `repr` do campo)
- Outros (`Float`, `Sum`, `List`, `Array`, `Function`): `kata_rt_repr_to_text`
  via FFI (delega para `pretty_print` do runtime, que caminha o `TypeShape`).

`repr` é parte do contrato `SHOW`, não do smart constructor. Um tipo pode ter
smart constructor sem `repr` (se não implementa `SHOW`), e pode ter `repr` sem
smart constructor (se o usuário define `show` manualmente).

**SHOW universal (post-refines):** A síntese de `show` é estendida para cobrir
**todos** os tipos, sem exceção:

- Structs com campos → `repr` sintetizado (caso existente).
- Enums → sintetizado.
- Refined (struct sem campos com `alias_of` e `predicates`) → sintetiza
  `show :: Refined => Text` chamando o show do tipo base (FFI direto, ex:
  `kata_rt_int_show` para base Int).
- Struct sem campos não-refined → sintetiza `show :: Struct => Text` com
  body `TextLit("StructName")` (representação trivial).
- Primitives (Int, Float, Text, Boolean, Rational) → `implements SHOW`
  manual na stdlib.
- Tipos com `implements SHOW` manual → já cobertos (skip na síntese).

`echo!(x)` funciona para qualquer `x` de qualquer tipo, inclusive refined,
sem `refines SHOW` e sem declaração do usuário. O `show_synthesis.rs`
verifica `has_manual_show` antes de sintetizar — implementação manual tem
prioridade.

#### 4.2.10. O Princípio Nominal-Estrutural ("Atrito Sadio")

* **Nas Fronteiras das Funções (Rigidez Nominal):** Se uma função espera
  `IdadeValida`, rejeita `Int` puro ou outro tipo refinado com os mesmos
  predicados. Dois refineds distintos sobre a mesma base são nominalmente
  incompatíveis entre si.
* **Nas Operações Base (Flexibilidade Estrutural via `refines`):** Um tipo
  refinado pode reutilizar as implementações de interface do seu tipo base
  mediante declaração explícita de `refines` (§4.4). Sem `refines`, o tipo
  refinado **não** interoperar com a base — `+ a 0` onde `a :: PositiveInt`
  falha. Com `PositiveInt refines NUM`, o fallback no dispatch substitui o
  refined pelo base e retenta. A interoperabilidade é opt-in, não automática.
* **Ascription é o caminho sem atrito:** prova compile-time para literais, sem
  `Result` e sem desempacotamento.
* **Downcast é a válvula explícita:** `a::Int` onde `a :: PositiveInt`
  rebaixa ao base sem custo em runtime (§4.2.7, modo 4). É a forma explícita
  de extrair a base, complementar ao `refines`.

### 4.3. Assinaturas e Tipos de Primeira Classe (`=>` vs `->`)

* **`=>` (Declaração):** Usado exclusivamente para firmar o contrato de uma
  função na sua definição. Argumentos à esquerda, retorno à direita, sem
  parênteses.
  * `soma :: Int Int => Int`

* **`->` (Tipo de Função):** Usado quando a assinatura de um lambda é tratada
  como tipo de dado transitável. Exige parênteses para desambiguar.
  * `map :: (A -> B) Iterable::A => Iterable::B`

### 4.4. `refines` — Delegação de Interface para Tipos Refinados

Tipos refinados (`data (Int, > _ 0) as PositiveInt`) podem reutilizar as
implementações de interface do seu tipo base mediante declaração explícita
de `refines`. O mecanismo é um fallback no dispatch — não cria overloads
sintetizados, não registra o tipo no `InterfaceRegistry`.

#### 4.4.1. Declaração

```kata
TipoRefinado refines INTERFACE
```

- `TipoRefinado` é um tipo refined declarado com `data (Base, predicados) as Nome`.
- `INTERFACE` é uma interface já declarada e já implementada pelo tipo base.
- Bloco indentado opcional com métodos parciais (caso misto, §4.4.4).

Sem bloco: delega todos os métodos da interface ao tipo base.

```kata
data (Int, > _ 0) as PositiveInt

PositiveInt refines NUM
```

Isso registra: "PositiveInt delega NUM, base é Int." Não registra PositiveInt
no `InterfaceRegistry` e não cria overloads no `DispatchTable`.

#### 4.4.2. Restrições

- `refines` só se aplica a tipos refined (`StructInfo` com `alias_of` e
  `predicates`). Aplicar a struct não-refined ou alias puro → erro compile-time.
- A interface deve já estar implementada pelo tipo base no `InterfaceRegistry`.
  Se o base não implementa → erro compile-time.
- O tipo base é resolvido seguindo `alias_of` no `StructRegistry`.
- `refines` não aceita `type_params` ou `iface_params` — refined types não são
  genéricos em 1.0.
- Um tipo pode refinar múltiplas interfaces: `PositiveInt refines NUM` e
  `PositiveInt refines SHOW` (embora SHOW seja automático, §4.2.9).

#### 4.4.3. Fallback no Dispatch

`refines` não cria overloads no `DispatchTable`. O mecanismo é um fallback
em `apply.rs`, executado quando o dispatch normal falha:

1. Verificar se **todos** os args refined são o **mesmo** tipo refined.
2. Consultar `refines_registry` para o tipo.
3. Verificar se `func_name` é método de alguma interface que o refined delega
   (percorrendo supertraits recursivamente).
4. Substituir todos os args refined pelo tipo base.
5. Retentar dispatch com os tipos base.
6. Se encontrado, examinar o tipo de retorno:
   - Se o retorno **implementa a interface** → passar pelo construtor falível
     do refined → `Result::(Refined, Err)`.
   - Se o retorno **não implementa a interface** → retornar direto, sem
     construtor.

O construtor é chamado **só** quando o tipo de retorno implementa a interface
sendo refinada. `PositiveInt refines NUM` (base Int, interface NUM):

| Método | Fallback | Retorno | Implementa NUM? | Resultado |
|---|---|---|---|---|
| `+` | `+ :: Int Int => Int` | Int | Sim | `PositiveInt(resultado)` → `Result::(PositiveInt, Err)` |
| `-` | `- :: Int Int => Int` | Int | Sim | `PositiveInt(resultado)` → `Result::(PositiveInt, Err)` |
| `<` | `< :: Int Int => Boolean` | Boolean | Não | `Boolean` direto |
| `=` | `= :: Int Int => Boolean` | Boolean | Não | `Boolean` direto |

O construtor falível avalia os predicados (`> _ 0`). Se o resultado viola o
predicado (ex: `- 1 5` = -4 para PositiveInt), o construtor retorna `Err`.
O usuário desempacota com `?` (Action) ou `match` explícito (função pura).

#### 4.4.4. Caso Misto (Partial Delegation)

```kata
PositiveInt refines NUM
    - :: PositiveInt PositiveInt => PositiveInt
        lambda a b:
            match (PositiveInt (- (a::Int) (b::Int)))
                Result::Ok(v): v
                otherwise: 0::PositiveInt
    # +, *, <, >, = delegados automaticamente
```

Método com corpo lambda = override explícito do usuário. Cria overload real
no `DispatchTable` (encontrado antes do fallback). Métodos não-listados usam
o fallback automático.

**Restrições do corpo lambda:** lambda é função pura — `?` (operador de
runtime, exclusivo de Actions) e `panic!` (Action builtin) não existem.
Para desempacotar `Result` em lambda, usar `match` explícito. O tipo de
retorno pode ser `PositiveInt?` (§4.5) se o override propaga falha, ou
`PositiveInt` nu se o override resolve o erro internamente (ex: clamp via
ascription literal `0::PositiveInt`).

O downcast `(a::Int)` (§4.2.7, modo 4) é usado no corpo para chamar a
implementação do base — `(- (a::Int) (b::Int))` despacha para
`- :: Int Int => Int`.

#### 4.4.5. Interoperabilidade com Tipos da Interface

Sem `refines`, PositiveInt **não interoperar** com Int. `+ a 0` onde
`a :: PositiveInt` e `0 :: Int` falha — não há overload e não há fallback.

Com `PositiveInt refines NUM`, o fallback passa a existir:

- `+ a b` onde `a :: PositiveInt, b :: Int`: fallback substitui PositiveInt
  por Int → `+ :: Int Int => Int` → encontrado. Retorno Int implementa NUM →
  construtor → `Result::(PositiveInt, Err)`.
- `+ a b` onde `a :: PositiveInt, b :: Float`: fallback substitui →
  `+ :: Int Float => ...` → não existe → falha.

A interoperabilidade é **opt-in** — o usuário declara intenção explicitamente.

#### 4.4.6. Incompatibilidade Nominal Entre Refineds Distintos

Dois tipos refined sobre a mesma base, mesmo com os mesmos predicados, são
**nominalmente incompatíveis**. O fallback só dispara quando **todos** os args
refined são o **mesmo** tipo.

```kata
data (Int, > _ 0) as PositiveInt
data (Int, > _ 0) as NonZeroInt

PositiveInt refines NUM
NonZeroInt refines NUM
```

`+ a b` onde `a :: PositiveInt` e `b :: NonZeroInt` → **falha**. Os refineds
são diferentes. O fallback não dispara. Para combinar refineds distintos, o
usuário faz downcast explícito: `+ (a::Int) b` ou `+ a (b::Int)`.

#### 4.4.7. Relação com `implements`

| | `implements` | `refines` |
|---|---|---|
| Quem usa | Qualquer tipo | Apenas tipos refined |
| Corpo | Usuário escreve | Fallback no typeck (ou override do usuário) |
| InterfaceRegistry | Registra | Não registra |
| Polimorfismo via interface | Sim | Não |
| DispatchTable | Cria overloads | Não cria (exceto override) |
| Retorno de métodos que devolvem tipo que implementa a interface | O que o usuário escrever | `Result::(Refined, Err)` via construtor |
| Retorno de métodos que devolvem tipo que não implementa a interface | O que o usuário escrever | Direto do base |

Um tipo pode ter ambos: `implements` para interfaces que define explicitamente,
`refines` para interfaces que delega ao base. PositiveInt não é formalmente
NUM no `InterfaceRegistry` — `soma :: T implements NUM => T T => T` não aceita
PositiveInt. O polimorfismo via interface fica para pós-1.0.

### 4.5. `T?` — Açúcar Sintático para `Result::(T)`

`T?` é açúcar puro de sintaxe de tipo. Lê-se "T ou falha". Desaçuca para
`Result::(T)` em todo lugar onde aparece um tipo: assinaturas de função,
tipos de retorno, tipos de campos, ascriptions.

`PositiveInt?` ≡ `Result::(PositiveInt)`. `Int?` ≡ `Result::(Int)`.

O tipo `Err` do `Result` tem um **default type param** declarado no prelude:
`Err(E=Text)`. Quando `Result` é instanciado com apenas 1 arg (como acontece
com `T?`), o default preenche `E=Text` automaticamente. Assim, o tipo final
efetivo é `Result::(T, Text)` — `Text` é o default, não hardcoded no açúcar.

```kata
soma_positiva :: PositiveInt PositiveInt => PositiveInt?
lambda a b: PositiveInt (+ a b)
```

Isso é apenas açúcar — o typeck resolve `PositiveInt?` para
`Result::(PositiveInt)` (1 arg), e o default `Err(E=Text)` do prelude
preenche `E=Text` antes de qualquer verificação.

#### 4.5.1. Default Type Params

A sintaxe `Err(E=Text)` no prelude declara que o type param `E` tem default
`Text`. Isto é **geral para qualquer enum com defaults**, não específico de
`Result`. O usuário pode declarar defaults em seus próprios enums:

```kata
enum Config
    Port(P=Int)
    Host(H=Text)
```

`Config::(Int)` resolve para `Config::(Int, Text)` via default. O usuário pode
instanciar com tipo customizado para não usar o default:
`Result::(Int, MyError)` funciona sem usar o default `E=Text`.

#### 4.5.2. O que `T?` não faz

- **Não cria subtyping.** `Int` não é subtipo de `Int?`. São tipos distintos.
- **Não cria Ok implícito.** Uma função que retorna `Int` não satisfaz
  `=> Int?` sem wrap explícito.
- **Não muda o operador `?` de runtime.** `?` em runtime continua sendo
  desempacotamento de `Result`. Não é no-op em não-Result.
- **Não habilita polimorfismo via interface.** PositiveInt continua não
  sendo NUM no `InterfaceRegistry`. `T?` é açúcar de escrita, não de semântica.

## 5. Os Domínios de Execução (Functions vs. Actions)

O compilador impõe uma barreira física e semântica inultrapassável entre código
puro e impuro.

### 5.1. Domínio Funcional (Functions)

Funções recebem dados, avaliam de forma estrita (eager) e devolvem dados.
* **Controlo de Fluxo:** Não existem *loops* imperativos (`for`/`while`) nem a
  palavra-chave `if`. O fluxo é gerido por *Pattern Matching* estrutural (cláusulas
  lambda múltiplas), *Guards* condicionais, e a palavra-chave `match`.
* **Tratamento de Falhas:** O operador `|` (fallback local) ou `|>` compondo
  lambdas de sucesso e falha. O operador `?` é **proibido** em funções puras.
* **Retorno:** Implícito — a última expressão do corpo é o retorno. Não existe
  `return` keyword nem `;` no domínio puro.

### 5.2. Domínio Imperativo (Actions)

As *Actions* interagem com o sistema operativo, o escalonador e a memória
mutável local.

**Declaração:** A definição da Action não usa `!` no nome. Params nomeados
usam `nome::Tipo` separados por vírgula; `=>` separa params de retorno:

```kata
action soma (a::Int, b::Int) => Int
    + a b

action conectar_servidor
    ...
```

- **Params nomeados:** `(a::Int, b::Int)` — `::` etiqueta nome→tipo, vírgula
  separa params. O body referencia `a`, `b` (não `__param_N`).
- **Sem params:** `action nome` ou `action nome => Unit`.
- **Retorno:** `=> Tipo` após os params (ou após o nome se sem params).
  Sem `=>` = retorno `Unit` (padrão).

**Chamada:** Toda chamada a *Action* exige obrigatoriamente o sufixo `!` e uma
tupla como argumento:

```kata
echo!("mensagem")
conectar_servidor!()
fork!(minha_action)
```

* **Proibição de Recursão:** O compilador aciona um Erro Fatal se detetar
  chamadas recursivas dentro de *Actions*, protegendo o ambiente contra *Stack
  Overflows* da corrotina.
* **Laços:** `loop` (infinito) e `for`, com `break` e `continue`.
* **Match statement:** `match` com pattern matching, guards condicionais e
  cláusula `otherwise`.
* **Propagação Monádica (`?`):** Extrai o valor de `Result` ou `Optional`. Em
  caso de erro, a *Action* aborta precocemente.

#### 5.2.1. Retorno e Caller's Arena

Actions suportam **retorno explícito** via keyword `return` e **retorno
implícito** (última expressão sem `;`):

```kata
action buscar
    let x := ler!()?
    match x
        Optional::Some(v): return v       # early return explícito
        Optional::None: return 0

action calcular
    let x := 5
    let y := + x 1
    y                                      # retorno implícito (última expr sem ;)

action greet
    echo!("hello")
    echo!("world");                        # ; → statement, action retorna Unit
```

**Por que `return` e `;` só existem em Actions:**

No domínio puro, `return` é **redundante com guards**. Guards já são o early
return das funções — `> x 0: x` retorna `x` se `x > 0`, sem necessidade de
keyword. E funções puras não têm fluxo imperativo para justificar early return:
toda recursão é estrutural via guards/pattern matching. O `;` também não faz
sentido no domínio puro — funções são expressões, não sequências de statements.
A última expressão é sempre o retorno.

No domínio impuro, `return` é **necessário** para early exit após `?` ou dentro
de `loop`/`match`. Sem `return`, toda action teria que estruturar seu fluxo para
que a última expressão fosse o valor desejado — impraticável com `?` (que pode
abortar no meio) e `break` (que pode sair de um loop).

**Caller's Arena — por que foi desenhado:**

Sem `return` explícito e caller's arena, Actions retornando coleções vazavam
ponteiros crus como `i64`. A causa: o typeck inferia `List(Int)` como retorno, o
codegen retornava o ponteiro cru, e a arena da action era destruída no epílogo
— use-after-free. A solução paliativa mapeava `List`/`Array` → `Unit` no
retorno de actions, descartando silenciosamente o valor.

A solução real é o **caller's arena**: quando uma Action retorna um
valor (via `return` ou retorno implícito), o compilador aloca o valor na **arena
do caller**, que persiste até o caller terminar. Isso permite que Actions
retornem coleções e estruturas sem use-after-free — a arena local da Action é
destruída no epílogo, mas o valor de retorno sobrevive na arena de quem chamou.

O `;` é o que distingue "computação local" de "valor que escapa". Sem `;`, o
compilador sabe que a expressão é um retorno e aloca na arena do caller. Com
`;`, a expressão é computação local na arena da Action, liberada no epílogo.

**`fork!` retorna `Unit`:** Actions submetidas via `fork!` comunicam
exclusivamente via canais. O fork não produz um valor de retorno síncrono.

### 5.2.2. Modelo de Memória

O Kata5 usa **arenas bump per-fiber** para toda alocação. Não há garbage
collector, não há reference counting, não há free individual. O modelo
funciona porque três restrições se combinam para garantir que todo
valor vive na arena certa e é liberado no momento certo.

#### As três arenas

| Arena | Tipo | Lifetime | O que vai aqui |
|---|---|---|---|
| **fiber_arena** | Bump (bumpalo) | Resetada quando o fiber morre | Valores locais que não escapam |
| **caller_arena** | Bump (bumpalo) | Resetada quando o caller morre | Valores que escapam via return ou canal |
| **root_arena** | Tracked (std::alloc) | Destruída no fim do processo | File handles, Result boxes de FFIs |

A **root_arena** é uma arena Tracked, mas não usa ARC — a função
`arena_alloc` aloca blocos individualmente rastreados (para permitir
`arena_dealloc` se necessário no futuro), mas não há incref/decref.
O cleanup é bulk no fim do processo.

#### A invariante que faz tudo funcionar: structured concurrency

O scheduler é **structured concurrency**: um fiber só é destruído quando
completa **E** todos os seus filhos completaram. Isto tem três
consequências que tornam as arenas bump suficientes:

1. **Valores que escapam via return ficam na caller_arena.** A action
   filha aloca o valor de retorno na arena do pai (caller_arena). O pai
   está vivo quando a filha retorna — o valor é válido.

2. **Valores que escapam via canal ficam na caller_arena.** A filha
   que envia aloca o valor na caller_arena (arena do pai). O pai está
   vivo enquanto houver filhas interessadas — o valor é válido quando
   o receptor o consome.

3. **Irmãos compartilham a caller_arena do pai.** Duas filhas do mesmo
   pai têm a mesma caller_arena (arena do pai). Valores trocados entre
   irmãos via canal são válidos porque o pai sobrevive a ambos.

Sem structured concurrency, este modelo quebra: se uma filha morresse
antes do pai, a arena dela seria resetada e valores enviados via canal
seriam use-after-free. A garantia de que o pai só morre depois das
filhas é o que fecha o sistema.

#### EscapeTarget — como o inference decide a arena

O inference atribui `EscapeTarget` a cada expressão:

- **`Local`** → `fiber_arena` (valor não escapa, morre com o fiber)
- **`Caller`** → `caller_arena` (valor escapa via return ou canal)
- **`Heap`** → `root_arena` (valor precisa de cleanup determinístico)

`Local` é atribuído quando o valor está em non-tail-position (não é
retorno). `Caller` quando está em tail-position (é retorno) ou quando
viaja por canal. `Heap` é reservado para recursos do SO (file handles).

O codegen usa `alloc_for_escape(escape, size, ctx)` que despacha para
a arena correta. O programador não vê nada disso — é totalmente
implícito.

#### I/O handles — a única exceção

File e Socket handles são recursos do SO (FDs) que precisam de close
determinístico — não podem esperar o fim do processo. O modelo:

- `open!` aloca `FileInner` ou `SocketInner` na root_arena via `arena_alloc`.
- `close!` faz `drop_in_place` (fecha o FD).
- O epílogo da action fecha automaticamente handles não fechados
  explicitamente (rastreados via `io_handle_vars` no codegen, generalização
  de `file_handle_vars` com `IoHandleKind::File`/`IoHandleKind::Socket`).
- O close é **idempotente** — o campo `closed` em `FileInner`/`SocketInner`
  garante que double-close (explícito + epílogo) é no-op, não double-free.

Todos os outros valores (Bytes, Text, Result boxes, listas, structs,
tuplas) são arena-allocated sem cleanup individual. A arena é
resetada em O(1) quando o fiber morre.

#### Por que não ARC

O Kata5 teve ARC (`alloc_tracked`/`incref_tracked`/`decref_tracked`)
entre as sessões 2-7. Foi removido na sessão 8 porque:

1. **Structured concurrency torna ARC desnecessário para canais.**
   A caller_arena cobre o lifetime de todos os valores que trafegam
   por canal. ARC era um mecanismo de segurança para um problema que
   não existe.

2. **`is_arc_type` era a pergunta errada.** ARC-ness é propriedade do
   local de alocação, não do tipo. O mesmo `Ty::List(Int)` pode ter
   sido alocado em arenas diferentes. Decidir incref/decref só pelo
   tipo causou SIGSEGV em `Byte` (SMI-tagged), UB silencioso em
   aliases de refined, e bugs latentes em vários tipos compostos.

3. **Bumpalo cresce dinamicamente.** Não há risco de estourar a
   arena. O risco é acumulação em fibers long-running, que se resolve
   com streaming (readline em loop) em vez de slurp.

4. **File handles só precisam de close, não de ARC.** O FD é um
   recurso do SO que precisa ser fechado. Close explícito + epílogo
   resolve sem o overhead e complexidade de ARC.

#### Implicações para o programador

- **Não há leak de memória em programas que terminam.** As arenas
  são resetadas quando os fibers morrem e a root_arena é destruída no
  fim do processo.

- **Fibers long-running (servidores) acumulam.** Valores alocados na
  fiber_arena de um servidor que nunca termina não são liberados. A
  resposta é usar streaming (readline, read_chunk) em vez de slurp,
  e structured concurrency para garantir que fibers auxiliares morram.

- **Close explícito é boa prática.** Embora o epílogo feche handles
  não fechados, chamar `close!` explicitamente libera o FD cedo —
  importante em servidores que abrem muitos arquivos.

- **Não há cópia na fronteira de return/canal.** O valor é alocado
  na arena certa desde o início (escape analysis). Sem memcpy.

### 5.3. Actions como Valores de Primeira Classe (First-Class Actions)

Actions podem ser **referenciadas sem invocação**, armazenadas em variáveis, e
passadas como parâmetros para outras Actions. Isto habilita o pattern de
dispatch/strategy:

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
    job!(payload)

action worker_a (n :: Int) => Unit
    echo!(+ n 1)

action worker_b (n :: Int) => Unit
    echo!(+ n 2)

action main => Unit
    dispatcher!(worker_a, 42)   # imprime 43
    dispatcher!(worker_b, 42)   # imprime 44

main!()
```

#### Referência vs Invocação

```kata
worker_a           # referência — valor do tipo Action(Int) => Unit
worker_a!(42)      # invocação — executa a action, retorna Unit
```

`worker_a` sem `!()` é uma **referência** que carrega o tipo `Ty::Action`. O
valor em runtime é o `fn_ptr` (i64) da Action — obtido via `GlobalValue::Symbol`
no codegen. O parser já produz `Expr::Ident` — não há mudança no parser. A
mudança é no typeck: `Ident` cujo nome está no DispatchTable com `is_action: true`
recebe `Ty::Action(param_types, ret_ty)`.

#### Sintaxe de Tipo

```
Action(Param1, Param2, ...) => Ret
```

Espelha a sintaxe de assinatura de actions, sem os nomes dos params:

```kata
action dispatcher (job :: Action(Int) => Unit, payload :: Int) => Unit
```

`Ty::Action` é separada de `Ty::Function` porque as ABIs são semanticamente
diferentes:
- `Function`: `(captures_ptr, arg1, ...) -> ret` — pura, sem scheduler
- `Action`: `(fiber_arena, caller_arena, args_ptr) -> i64` — impura, scheduler cooperativo

#### Passagem como Parâmetro

Dentro de `dispatcher`, `job` é um parâmetro com `ty: Ty::Action([Int], Unit)`.
A invocação `job!(payload)` é **indireta** — o fn_ptr vem do parâmetro, não de
um nome estático. O codegen emite `call_indirect` com a ABI de Action.

#### Seleção por `match`

Actions podem ser selecionadas em runtime via `match`:

```kata
action main => Unit
    let cond := True
    let f := match cond
        Boolean::True: worker_a
        Boolean::False: worker_b
    f!(42)   # invoca worker_a indiretamente
```

O def-use do recursion checker registra arestas conservativas para ambas as
actions do `match` (worker_a e worker_b).

#### `fork!` com Action como valor

`fork!` recebe um `TypedExpr` com `ty: Ty::Action`. Se o arg é `Ident` direto,
o codegen usa `GlobalValue::Symbol`. Se é variável (`let f := worker`), usa
o fn_ptr da variável:

```kata
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    let f := worker
    fork!(f, (42,))    # fn_ptr vem da variável f
```

#### Restrições

| Operação | Status | Racional |
|---|---|---|
| `let f := worker_a` | ✅ Permitido | Binding direto — nome rastreável |
| `dispatcher!(worker_a, 42)` | ✅ Permitido | Action como param de Action |
| `f!(42)` onde `f` é param | ✅ Permitido | Invocação indireta |
| `fork!(f, (42,))` onde `f := worker` | ✅ Permitido | Fork recebe Action como valor |
| Action como campo de `data` | ❌ Proibido | `data` é reino de dados, não de comportamento |
| Action via canal | ❌ Proibido | Canais transportam dados, não comportamento |
| Action como parâmetro de função pura | ❌ Proibido | Funções puras não podem invocar actions |
| Interface `CALLABLE` | ❌ Não existe | Functions e Actions são reinos separados |

O typeck rejeita `Ty::Action` em posições de `data`, canal, e função pura.
Actions são **importáveis** (via `import`), não precisam ser transportadas
por canais ou armazenadas em structs.

#### Recursion Check — Def-Use Interprocedural

Quando `dispatcher!(worker_a, 42)` passa `worker_a` como param `job` e
`dispatcher` invoca `job!(payload)`, o recursion checker registra a aresta
`dispatcher → worker_a`. Se `worker_a` invoca `dispatcher` (direta ou
indiretamente), o ciclo é detectado. O algoritmo propaga nomes literais dos
call sites para os params que eles preenchem, transitivamente até fixpoint.

#### Tree Shaking

Uma action referenciada como first-class value (`Ident { ty: Action }`) é
marcada como alcançável pelo tree shaker — pode ser invocada indiretamente.
Uma action não referenciada (nem invocada, nem referenciada como valor) é
removida.

## 6. Concorrência e Gestão de Memória (Modelo CSP)

O runtime fornece escalonamento **cooperativo single-threaded** — M fibers
(tasks Kata) em 1 thread OS, com round-robin e yield cooperativo. Multithread
M:N (worker pool com work-stealing) é aspiracional, planejado para post-1.0.

### 6.1. Scheduler como Struct Explícita

O scheduler é uma struct explícita, não globais TLS:

```rust
pub struct Scheduler {
    run_queue: VecDeque<FiberId>,
    blocked: HashMap<FiberId, BlockReason>,
    pending_wakes: HashSet<FiberId>,
    timers: TimerQueue,
    current_fiber: Option<FiberId>,
}
```

TLS é usado apenas para o `yield` (acesso implícito de dentro de FFI). O
scheduler em si é parâmetro explícito. Isso torna testes isolados triviais e
desbloqueia multithread sem refatoração (post-1.0).

### 6.2. Fibers e Yielding Cooperativo

Uma *Action* não bloqueia a thread OS. Ao invocar operações bloqueantes como
`<!` (receber do canal), a fiber cede o controlo (yield) para o scheduler, que
retoma a próxima fiber pronta da `run_queue`. Quando o dado chega, a fiber
original é acordada.

### 6.3. Topologias de Canais

O *fork!* submete processos sequenciais isolados que comunicam por três vias:
1.  `channel!`: Síncrono (Rendezvous). O envio `!>` bloqueia até o recetor `<!`
    sincronizar.
2.  `queue!(N)`: Fila com buffer de `N` espaços (Backpressure).
3.  `broadcast!`: Difusão 1-para-N (Publish-Subscribe).

Canais usam `Mutex`/`Condvar` (futex no Linux) para blocking cooperativo.
O scheduler é single-threaded — os locks garantem consistência para o
wake pass do scheduler, não para concorrência entre threads.

Para orquestrar múltiplas vias, utiliza-se a estrutura `select` com casos de
`timeout`, que multiplexa eventos sem inanição (*starvation*).

### 6.4. `spawn!` (Multiprocess)

`spawn!` é um special form ao lado de `fork!` que executa uma Action num
**processo OS separado** via fork + IPC. Isso oferece isolamento total e
paralelismo real para CPU-bound pesado — o scheduler de fibers não gerencia
isso; é uma ponte diferente:

```
spawn!(action, args) → fork processo → IPC channel → resultado
```

**Sintaxe — duas formas:**

```
spawn!(tarefa, (42, arr))                        # posicional — runtime serializa
spawn!{callee: tarefa, raw: (42, arr)}            # dict — runtime serializa
spawn!{callee: tarefa, serialized: payload}      # dict — bytes pré-serializados
```

A forma posicional é açúcar para `spawn!{callee: tarefa, raw: args}`. A chave
`serialized:` aceita um blob produzido por `to_bytes()` (FFI do runtime) e
envia os bytes direto, sem re-serialização.

**Marshalling:** entre processos, valores são sempre by-value (serializados).
Tipos primitivos (SMI, Float) são copiados direto (8 bytes). Tipos heap-allocated
(Text, Array, Dict, tuplas, structs, enums) são serializados recursivamente via
`TypeShape` walk — a mesma estrutura que o decref walk percorre. O custo é
proporcional ao tamanho dos dados.

O modelo é análogo ao de Erlang/BEAM: processos leves (fibers) para concorrência
(CSP), processos OS para isolamento/paralelismo pesado.

### 6.5. Memória: Arenas O(1), Caller's Arena e ARC

Como não há Garbage Collector, a posse da memória é regida em tempo de compilação:

* **Arena local (per-fiber):** Dados criados numa *Action* são alocados num bump
  allocator local. Quando a fiber termina, a Arena é libertada em O(1). Não há
  sharing entre threads — cada fiber tem sua arena.
* **Caller's Arena:** Valores de retorno de Actions são alocados na arena do
  caller (zero cópia, persiste até o caller terminar).
* **Heap (Root Arena) + ARC manual:** Dados que escapam por canais são
  alocados na root arena (TrackedArena) com ARC manual — um CaptureBox com
  header próprio (`fn_ptr`, `refcount`, `n_captures`). O compilador injeta
  `incref`/`decref` via FFI. Quando o refcount chega a 0, `kata_rt_decref`
  libera o bloco individualmente da root arena. O refcount é não-atomic
  (adequado ao scheduler single-threaded).

## 7. Diretivas de Compilador (@)

Anotações sintáticas que instruem o compilador a alterar comportamentos
não-funcionais. Aplicam-se a *actions*, *lambdas* e *data* (itens de topo de
módulo), sempre precedendo imediatamente o item que modificam.

### 7.1. Sintaxe Geral

```
@nome                     # diretiva sem argumentos
@nome("argumento")        # diretiva com 1 argumento posicional anônimo
@nome{chave: valor, ...}  # diretiva com argumentos nomeados entre chaves
```

### 7.2. Catálogo de Diretivas

* **`spawn!`**: Executa uma *Action* num **processo OS separado** via fork + IPC.
  Ideal para uso massivo de CPU com isolamento total. Aceita tupla (serializa
  implicitamente) ou dict com `raw:`/`serialized:` (controle explícito).
  Veja seção 6.4.
* **`@comptime`**: Avalia uma expressão durante a compilação via JIT-and-execute
  (compila a expressão usando o pipeline normal e executa no `kata-rt` real),
  substituindo o resultado por um literal na TAST. Tem duas formas de uso:
  - **Top-level (Fase 1-2 ✅):** `@comptime` antes de um `let` top-level ou
    expressão top-level. O comptime pass avalia a expressão e substitui o
    resultado por um literal (escalares: Int, Float, Boolean, Unit) ou por
    um `HeapSnapshot` (tipos complexos: List, Tuple, Struct, Text, Sum com
    payload). O snapshot é serializado de forma type-aware (incluindo
    Sum/Result com payload Text — strings e payloads são copiados
    recursivamente para o snapshot, não ponteiros crus), embutido como
    data symbol no binário JIT, e carregado na root_arena em load-time via
    `kata_rt_load_snapshot` com rebasing de ponteiros. Exemplo:
    `@comptime let x := [1 2 3]` gera `x` como snapshot navegável em runtime.
  - **Call-site (guarantee) — implementado:** `@comptime` antes de uma
    expressão dentro de um body força avaliação em compile-time. Se
    consegue, substitui por snapshot; se não consegue, erro de compilação.
    `@comptime` envolve apenas a aplicação greedy (callee + args) —
    pipe (`|>`), fallback (`|`), e canais (`!>`, `<!`) ficam fora do
    escopo. Para incluí-los, use parênteses: `@comptime (x |> f)`.
    Bindings locais avaliados via `@comptime let` são comptime-available
    para `@comptime` posterior no mesmo body.
    (Definition-site `@comptime` foi removido do escopo — a decisão de
    avaliar pertence ao call-site, onde os args são visíveis.)
* **`@cache_strategy{strategy: "LRU"}`**: Interceta invocações puras repetidas e
  injeta pesquisas em Hash Table nativa (ex: `LRU` cache), efetuando
  *memoização* automática.
* **`@test("descrição")`**: Marca um bloco para o *Test Runner*. A forma braced
  `@test{desc: "...", expects: "CompileError"}` marca um **teste negativo** —
  verifica que o código **não compila**.
* **`@log{msg: "...", when: "enter"|"exit", level: LogLevel::Info, topic: "audit", policy: "block"}`**: Injeta
  telemetria via canais CSP. `msg` é template compile-time com interpolação
  `{expr}`. `when` obrigatório: `"enter"` loga no prólogo, `"exit"` no epílogo.
  Política `"drop"` = Broadcast fire-and-forget; `"block"` = Queue com
  backpressure. Independente de `log!()` (action nativa para publicação
  explícita no corpo). Ver §20.
* **`@associative(0)`**: Anota que a função é associativa, informando o elemento
  neutro. Permite que o otimizador TRMA converta recursões bloqueadas em
  recursão de cauda perfeita.
* **`@commutative`**: Anota que a função é comutativa. O algoritmo de despacho
  múltiplo tentará argumentos invertidos ao procurar sobrecargas compatíveis.
* **`@ffi("nome_simbolo_c")`**: Ponte para símbolo C nativo. Informa ao *Linker*
  que o corpo não será fornecido pelo código Kata, mas importado de biblioteca
  externa. (Ver seção 21.)
* **`@builtin("tag")`**: Marca uma função como builtin sintetizado. O
  compilador gera nós TAST especializados (ex: `Map`, `Filter`, `Fold`) em vez
  de chamadas de função. Usado na stdlib para `map`/`filter`/`fold`.

  **Padrão completo de interceptação:**

  1. A função é definida na stdlib em código Kata com assinatura tipada e
     diretiva `@builtin("nome")`.
  2. O typeck reconhece a diretiva e gera nó TAST próprio (`Map { func,
     iterable }`, `Filter`, `Fold`) em vez de `Apply` comum.
  3. O body em Kata é ignorado — o lowering é especializado por tipo concreto
     (List → head/tail, Array → index/len, Range → iteração por índice).
  4. O usuário pode definir funções com outros nomes sem conflito; a primitiva
     só é ativada pela diretiva.

  Isso separa "o compilador conhece isto" de "o compilador implementa isto." A
  função existe na stdlib com assinatura tipada; o compilador conhece a diretiva
  e decide como lowerar. Este padrão deveria ser usado consistentemente para
  qualquer função que o compilador precise interceptar — não adicionar
  interceptação ad hoc no typeck.

## 8. Estruturas de Coleções e Iteração (A Interface `ITERABLE`)

A linguagem separa estritamente os tipos de dados básicos das coleções, e o
*layout* de memória dessas coleções é ditado diretamente pelos delimitadores
sintáticos no momento da declaração.

### 8.1. Topologias de Memória e Sintaxe

* **Listas Persistentes (`[T]`):** Sintaxe `[ ]`. Topologia encadeada (Cons de
  cabeça e cauda) para imutabilidade de custo zero via partilha estrutural.
* **Arrays Contíguos (`{T}`):** Sintaxe `{ }`. Bloco contíguo para maximizar
  cache da CPU e iterações imperativas eficientes.
* **Dicionários (`Dict::(K, V)`):** Sintaxe `{"k": v}`. Mapeamento persistente
  imutável baseado em Hash Array Mapped Trie (HAMT) com sharing estrutural.
  Alocado na arena per-fiber. `K` deve implementar `HASHABLE`. `{:}` para vazio.
  O `:` após a primeira entrada desambigua de Array. Mantém ordem de inserção
  via Cons-list overlay — `replace = nova inserção` (chave move para o fim).
* **Conjuntos (`Set::T`):** Sintaxe `{|1 2 3|}`. Conjunto persistente imutável
  baseado em HAMT (delega para Dict com `Unit` como valor). `T` deve implementar
  `HASHABLE`. `{||}` para vazio. Não mantém ordem de inserção (iteração via
  HAMT, não determinística).
* **Tensores N-Dimensionais (`{T::Int...}`):** Separador de dimensão `;` dentro
  de `{}`. Dimensionalidade processada em compile-time (*Const Generics*).
* **Ranges (Lazy):** `[0..10]` (0 a 9), `[0..=9]` (0 a 9 incluso),
  `[0..2..10]` (0 a 9 com step 2). Geram `Range` lazy que implementa `ITERABLE`.
  O step é definido por um segundo `..`: `start..step..end`.

### 8.2. Interfaces de Coleção

As coleções implementam interfaces parametrizadas que definem contratos
uniformes para operações comuns:

**`ITERABLE(A)`** — Iteração:
```kata
interface ITERABLE(A)
    next :: Self => Optional(A)
```

**`COUNTABLE`** — Tamanho:
```kata
interface COUNTABLE
    len :: Self => Int
```

**`INDEXABLE(A)`** — Acesso posicional:
```kata
interface INDEXABLE(A)
    at :: Self Int => Result::(A, Err)
```

**`CONTAINS(A)`** — Pertinência:
```kata
interface CONTAINS(A)
    contains :: Self A => Boolean
```

**`HASHABLE`** — Hash semântico (pré-requisito para Dict/Set):
```kata
interface HASHABLE
    hash :: Self => Int
```

`HASHABLE` é necessária porque o runtime trata valores como `i64` opaco —
não tem type info para hashear conteúdo. Valores unboxed (SMI Int, Float
bitcast) são automaticamente hashable pelo bit pattern; valores boxed (Text,
Struct, Tuple, Sum) precisam de hash semântico que conhece o tipo. `Int`,
`Text`, e `Rational` implementam `HASHABLE` na stdlib.

Implementações na stdlib:

| Tipo | ITERABLE | COUNTABLE | INDEXABLE | CONTAINS | HASHABLE |
|---|---|---|---|---|---|
| `Array(A)` | ✅ | ✅ (`kata_rt_array_len`) | ✅ (`kata_rt_array_get_checked`) | ✅ | — |
| `List(A)` | ✅ | ✅ (traversal stdlib) | ✅ (traversal stdlib) | ✅ | — |
| `Text` | ✅ | ✅ (`kata_rt_string_len`) | ✅ (`kata_rt_string_get_checked`) | ✅ | ✅ |
| `Range` | ✅ | ✅ (compile-time) | — | — | — |
| `Dict::(K, V)` | ✅ | ✅ (`kata_rt_dict_len`) | ✅ (`kata_rt_dict_get_checked`, chave) | ✅ (`kata_rt_dict_contains`) | K deve implementar |
| `Set::T` | ✅ | ✅ (`kata_rt_set_len`) | — | ✅ (`kata_rt_set_contains`) | T deve implementar |
| `Tuple` | — | special case (síntese) | special case (compile-time) | — | — |

Tuple não implementa interfaces — é um tipo estrutural, não nominal. `len` e
`.N` em Tuple são special cases do typeck (ver §14.3).

### 8.3. Operadores de Coleção

O operador `+` é sobrecarregado na stdlib para coleções além de `NUM`:

| Assinatura | Operação | FFI |
|---|---|---|
| `+ :: List::A List::A => List::A` | Concatenação | `kata_rt_list_concat` |
| `+ :: Set::T Set::T => Set::T` | União | `kata_rt_set_union` |
| `+ :: Set::T T => Set::T` | Inserção | `kata_rt_set_insert` |
| `+ :: Dict::(K, V) Dict::(K, V) => Dict::(K, V)` | Merge (right-biased) | `kata_rt_dict_merge` |

O operador `-` para `Set` e `Dict`:

| Assinatura | Operação | FFI |
|---|---|---|
| `- :: Set::T Set::T => Set::T` | Diferença | `kata_rt_set_difference` |
| `- :: Set::T T => Set::T` | Remoção | `kata_rt_set_remove` |
| `- :: Dict::(K, V) K => Dict::(K, V)` | Remoção | `kata_rt_dict_remove` |

**Dict `+` merge é right-biased:** `+ d1 d2` insere cada (k, v) de `d2` em
`d1`. Em conflito de chaves, o valor de `d2` vence. `insert` é a operação
para adicionar uma única chave — `+` para Dict é merge, não insert.

**Kata é prefix-only:** `+ s t` (aplicação prefix), nunca `s + t` (infix).
`+` e `-` são `Ident(String)` tokens — o parser trata como nomes de função.
O dispatch resolve por tipo dos argumentos: `+ set1 set2` é união, `+ set
elem` é inserção, sem ambiguidade.

### 8.4. Polimorfismo e Stream Fusion

A stdlib usa `@builtin("map")`, `@builtin("filter")`, `@builtin("fold")` para
gerar nós TAST estruturados que o optimizer pode fusionar (StreamFusion) —
`map(f, filter(g, arr))` vira um único loop sem coleções intermediárias.

## 9. Tipos Algébricos de Dados (ADTs)

A linguagem rejeita classes e herança orientada a objetos. A modelagem é por
ADTs sem métodos acoplados.

### 9.1. Tipos Produto (`data`)

Conjunção lógica (AND), bloco contíguo alocado (Struct). Declaração posicional
em parênteses ou formatação indentada. Tipagem dos campos via `::`.

### 9.2. Tipos Soma (`enum`) e Variantes Predicadas

Disjunção lógica (OR), dimensionados pelo tamanho da maior variante + discriminant
tag. Variantes listadas em linhas indentadas — **não** se usa `|` para separar.
A qualificação `Enum::Variante` é sempre válida.

**Invariante de codegen: Sum com payload é sempre ponteiro (box 8 bytes).**
Em vez de ter Sum inline para variantes pequenas e Sum boxed para grandes, todo
Sum com payload é uniformemente um ponteiro. Esta decisão (TD-3)
simplifica o codegen a custo de uma indirection extra em todo acesso de variante.
Os sites de extract usam `is_shared`/`is_arena_value` para distinguir o tipo de
ponteiro. Não tentar otimizar Sum inline para variantes pequenas sem revisitar
todos os sites de extract.

Variantes podem carregar **valores fixos** ou **predicados lógicos**:

```kata
enum StatusHTTP
    OK(200)
    Created(201)
    BadRequest(400)

enum IMC
    Magreza(< _ 18.5)
    Normal(<= _ 25.0)
    Sobrepeso(<= _ 30.0)
    Obesidade           # default (unitária)
```

Regras: se uma variante tem predicado, todas (exceto a última) devem ter. A
última é o *fallback* sem predicado. O construtor inteligente avalia de cima
para baixo; o primeiro predicado satisfeito captura o valor.

## 10. Gestão de Erros e Padrões Funcionais Puros

A Kata-Lang condena `null/nil` e exceções `try/catch`. O controlo de falhas faz
parte da assinatura do sistema de tipos.

### 10.1. A Ausência Segura (`Optional::T`)

Quando o resultado pode não existir logicamente sem constituir falha fatal, a
função retorna `Optional`, forçando verificação de `Some(T)` ou `None`.

### 10.2. A Falha Processual (`Result::(T, E)`)

Falhas calculáveis devolvem `Result` (`Ok(T)` ou `Err(E)`), definido na stdlib.
No domínio impuro, `?` delega o erro ao orquestrador.

### 10.3. O Crash Determinístico (`panic!`)

```kata
panic!("mensagem de erro")           # aborta imediatamente
```

`panic!` é uma Action builtin que aborta a execução imediatamente, escreve a
mensagem no stderr e chama `exit(1)`. Destrói a Arena local da *Action*. Retorna
`Unit` no sistema de tipos — mas o fluxo nunca chega ao retorno.

`panic!` não desempacota nem valida — é para estados de corrupção onde continuar
seria incorreto. O tempo de vida da arena é descartado; não há cleanup gracioso.

### 10.3.1. Asserções (`assert!`)

```kata
assert!(cond)                        # 1 arg: panic!("assertion failed")
assert!(cond, "mensagem customizada") # 2 args: panic!(msg)
```

`assert!` é uma Action builtin que desugara para `match` no typeck:

```kata
# assert!(cond, "msg") desugara para:
match cond
    Boolean::True: Unit
    Boolean::False: panic!("msg")
```

Com 1 argumento, a mensagem default é `"assertion failed"`. Com 2 argumentos,
o segundo é a mensagem de erro. O `cond` deve avaliar para `Boolean`.

`assert!` é lowerado para `Guard` com `Panic` no fallback — código inline, não
rastreável em nível de função. Em `kata build`, tree shaking elimina `@test`
mas `assert!` sobrevive (não é diretiva `@test`).

### 10.4. Resolução de Falhas: Delegação (`?`) vs. Contenção (`|`)

| Funcionalidade | `?` (Sufixo) | `\|` (Infixo) |
|:---|:---|:---|
| **Filosofia** | "Se falhar, aborto. Problema do chamador." | "Se falhar, contingência assume localmente." |
| **Domínio** | Estritamente Actions. | Functions e Actions. |
| **Fluxo** | Interrompe e retorna `Err(e)`. | Desempacota payload da variante não-cauda; se cauda, avalia rhs. |
| **Compatível com** | `Result`, `Optional` (e futuros enums com variante de erro). | `Optional` e qualquer enum cuja última variante seja **unitária** (cauda sem payload). |
| **Incompatível** | Funções puras. | `Result` (`Err` tem payload — não é cauda unitária). Enums sem cauda unitária. |

**`|` (fallback local)** é um operador infixo que desempacota o payload de
qualquer variante não-cauda. Se a expressão à esquerda é a cauda (última
variante, unitária), avalia e retorna a direita. Foi generalizado via
`EnumRegistry` no typeck — funciona com qualquer enum cujas variantes (exceto a
última) carreguem payload e a última seja unitária:

- `Optional::Some(v) \| default` → desempacota `v`; `Optional::None \| default`
  → avalia `default` (None é cauda unitária). ✅
- `Result::Ok(v) \| 0` → **type error**: `Err(E)` tem payload, não é cauda
  unitária. Use `?` para fail-fast. ❌
- User enums com cauda unitária ganham `\|` automaticamente (ex: `enum Light
  { On(bool), Off }` → `Light::On(true) \| Light::Off` desempacota `true`).

O `|` é desugared para `Match` no typeck — a TAST nunca contém `PipeFallback`.

**Coerção contextual no `|`:** Se o payload da variante não-cauda é um tipo
refinado e o fallback é um literal do tipo base, o compilador valida os
predicados do fallback em compile-time (coerção contextual). Isto aplica-se
aos enums compatíveis com `|` (cauda unitária), não a `Result`.

## 11. Infraestrutura de Testes (*Low-Cost Abstraction*)

Testes não precisam de bibliotecas externas; são validados e podados pelo motor
de compilação.

* **`@test("descrição")`:** Marca blocos para o *Test Runner*. Em `kata build`,
  tree shaking elimina estes blocos do binário de produção.
* **Testes Negativos (`expects: "CompileError"`):** Verificam que o código **não
  compila**. O type checker processa cada função/action individualmente; quando
  um teste negativo falha a inferência, é omitido do `TypedModule` sem interromper
  o typeck do restante. O *Test Runner* detecta a ausência e reporta **PASS**.
* **Tree Shaking:** Executado incondicionalmente (não exige `--release`).
  Elimina `@test` e código não-alcançável. `assert!` é lowerado para `Guard` com
  `Panic` no fallback — código inline não rastreável em nível de função.

### 11.1. Tree Shaking (Dead Code Elimination)

3 fases a partir das Actions como entry points:
1. **Collect:** Worklist iterativa, expande transitivamente. Overloads tratados
   em grupo.
2. **Mark:** Seta `reachable` nas entidades.
3. **Filter:** Remove não-alcançáveis. **Signatures sempre preservadas.** Actions
   nunca eliminadas.

## 12. O Padrão *Newtype* e a Palavra-chave `alias`

`alias` cria um **Novo Tipo Nominal Forte** (*Newtype*). Distinto do original
para o type checker, mas com custo zero em runtime (invólucro eliminado).

Todo alias herda um construtor baseado no tipo de origem, preservando a semântica
de falha (se o base retorna `Result`, o alias também).

O principal motivo de existência do `alias` é resolver a **Regra de Coerência
(Orphan Rule)** — permite implementar uma interface externa num tipo externo
encapsulando-o localmente.

## 13. Variantes Predicadas e Valores Fixos em Tipos Soma (`enum`)

Variantes podem carregar valores fixos (`OK(200)`) ou predicados (`Magreza(< _
18.5)`). O construtor inteligente injeta validação invisivelmente, despachando
para a variante cujo predicado satisfaz.

## 14. A Teoria Unificada das Tuplas e Notação Prefixa

Notação prefixada estrita (`+ 1 1` em vez de `1 + 1`). Elimina precedência de
operadores e torna o parsing linear.

#### Espaços Significativos

`+ 1 1` é válido; `+1 1` não é (lexer entende `+1` como número positivo ou nome
de função). Liberta `+` e `-` para uso com números (`-10` = dez negativo).

#### Parênteses: Agrupamento e Tuplos

**Vírgula define tuplo vs agrupamento.** Tem vírgula = tuplo; não tem =
agrupamento.

Esta regra (DT-4) resolve uma ambiguidade fundamental da notação
prefixa: sem ela, `(+ 1 2)` poderia ser aplicação de função **ou** tupla de 3
elementos. A vírgula como marcador de tuplo é a forma mais simples de
desambiguar — o parser não precisa de lookahead semântico, só verifica se há
vírgula após o primeiro elemento.

* `(expr)` = agrupamento (transparente ao typeck). `(42)` é o literal `42`, não
  tupla. `(+ 1 2)` é aplicação de função, não tupla.
* `(a, b, c)` = `Tuple([a, b, c])`. Vírgula obrigatória.
* `(1,)` = tuplo de 1 elemento (vírgula obrigatória). Sem vírgula, `(1)` é
  agrupamento.
* `()` = `Unit` (zero-sized, tupla vazia).
* `$ tuplo` = spread (interceptado pelo typeck, não builtin — ver §19.3).

**Invariante de codegen: Tuple é sempre heap type (ponteiro).** Assim como Sum
com payload (§9.2), toda Tuple é alocada na arena como um bloco contíguo de
words (8 bytes por elemento). O codegen faz `arena_alloc` + `Store` por
elemento no lowering da tupla, e `Load` por offset no acesso posicional. Esta
uniformização simplifica o codegen — não há caso inline vs heap para Tuple,
sempre é ponteiro. `is_heap_type()` retorna `true` para Tuple.

#### Acesso Posicional (`.N`)

A sintaxe `expr.N` funciona em tuplas e coleções. O comportamento é **uniforme
na sintaxe, distinto no tipo de retorno** — seguindo o princípio do atrito
sadio: fricção onde há risco, não onde há prova.

**Tuplas — retorno direto (compile-time safe):**

```kata
let t := (10, 20, 30)
t.0          # 10 — tipo Int (direto, sem Result)
t.2          # 30 — tipo Int
t.(-1)       # 30 — último elemento (índice negativo conta do fim)
t.5          # type error: IndexOutOfBounds (verificado em compile-time)
```

O typeck conhece o tamanho da tupla (estático no tipo `Tuple([T])`). Bounds
check é compile-time. Índices negativos são resolvidos estaticamente:
`t.(-1)` = `t.(len-1)`. O tipo do resultado é o tipo da posição — direto, sem
`Result`, sem `?`, sem friction.

**Coleções — retorno `Result` (runtime safe):**

```kata
let arr := {1 2 3}
arr.0          # desugar → at arr 0      → Result::(Int, Err)
arr.(-1)       # desugar → at arr (-1)   → Result::(Int, Err), runtime resolve negativo
arr.0 ?        # desugar → (at arr 0) ?   → Int (unwrap or propagate)
arr.0 | 0      # desugar → (at arr 0) | 0 → Int (fallback)

let lst := [1 2 3]
lst.1 ?        # desugar → (at lst 1) ?   → Int (List implementa INDEXABLE, O(n) traversal)
```

Para coleções, `.N` é **syntactic sugar para `at`** (interface `INDEXABLE(A)`).
O typeck faz o desugar baseado no tipo do receptor: se implementa `INDEXABLE`,
`.N` vira `at obj N`, retornando `Result::(A, Err>`. O programador usa `?` ou
`|` para desempacotar — mesma friction de qualquer operação com risco de
runtime.

**Índice negativo:**

Índices negativos contam do fim: `.(-1)` = último, `.(-2)` = penúltimo. Para
tuplas, resolvido em compile-time (`t.(-1)` = `t.(len-1)`). Para coleções,
resolvido em runtime (`at` recebe o índice negativo e o runtime ajusta).

**Desugar no typeck:**

| Receptor | Tipo de retorno | Mecanismo |
|---|---|---|
| `Tuple([T])` | `T_N` direto | `IndexAccess` — compile-time bounds check |
| Tipo implementa `INDEXABLE(A)` | `Result::(A, Err>` | Desugar para `at obj N` |
| Outro | — | `NotIndexable` error |

**Por que Tuple é direto e coleções são Result:**

O typeck pode provar que `t.0` em `(10, 20, 30)` é seguro — o tamanho é 3, o
índice é 0. Retornar `Result` aqui seria false friction, como fazer `42 + 1`
retornar `Result::(Int, OverflowErr>` "just in case." Para coleções, o tamanho
só é conhecido em runtime — não há prova compile-time, então `Result` é o atrito
correto. A distinção é a mesma de `/` (exato, exige `NonZero`) vs `div`
(dinâmico, retorna `Result`) — ver §22.

**Invariante de codegen: Tuple é heap type.** Toda Tuple é alocada na arena como
ponteiro (bloco contíguo de words). Acesso por índice é `Load` por offset — ver
§14.2.

**Sintaxe `.`:** Mesma sintaxe para field access em structs (`pessoa.nome`) e
indexação (`t.0`, `arr.1`). O parser aceita `Ident` ou `Int` após `.`. O typeck
resolve pelo tipo do receptor: struct → field access, tupla → IndexAccess
compile-time, INDEXABLE → desugar para `at`, outro → `NotIndexable`.

#### `len` (Tamanho)

A função `len` retorna o número de elementos. Uniforme na sintaxe, distinta no
mecanismo:

**Tuplas — síntese compile-time (zero-cost):**

```kata
len (10, 20, 30)     # 3 — IntLiteral, nunca chega ao codegen
```

`len` em Tuple é special case do typeck: vê `Tuple([T])`, conta
`element_types.len()`, emite `IntLiteral`. Zero-cost — o runtime nunca vê esta
operação.

**Coleções — dispatch via interface `COUNTABLE`:**

```kata
len {1 2 3}          # 3 — at via kata_rt_array_len (FFI)
len [1 2 3]          # 3 — traversal stdlib
len "hello"          # 5 — kata_rt_string_len (FFI)
```

`len` em coleções é dispatch via interface `COUNTABLE` — `len :: Self => Int`.
Cada tipo implementa com o mecanismo apropriado: Array e Text via FFI, List via
traversal stdlib.

## 15. Controlo de Escopo: `let`, `var` e `with`

* **`let`:** Imutável. Padrão da linguagem. Único permitido em funções puras.
* **`var`:** Mutável. Exclusivo de Actions. Mutação na referência da stack da
  corrotina, nunca nos dados imutáveis da Arena.
* **`with`:** Bloco bottom-up no final de lambda. Computações prévias para Guards
  e restrições de genéricos (`T implements ORD`).

## 16. Condicionais Puras: Guards e Pattern Matching

Sem `if/else`, a Kata-Lang usa pattern matching estrutural e guards condicionais.

* **Pattern Matching:** Destrincha a "forma" do dado. Não realiza computação.
* **Guards:** Testes computacionais booleanos. Separados por `:`.
* **Curto-circuito:** Avaliação de cima para baixo. Primeiro guard verdadeiro
  retorna.
* **`otherwise:`** Fallback mandatário no fim de qualquer corrente de Guards. Sem guards, o body direto dispensa `otherwise`.

### 16.1 `with` Block — Computações Prévias

`with` é um bloco de bindings nomeados que aparece **depois dos guards** (ou do
body direto, quando não há guards) no fim da cláusula lambda (como `where` em
Haskell). Os bindings são visíveis em **todos os guards da cláusula**, mesmo
sendo escritos depois — a ordem é visual
(legibilidade), a semântica é que os bindings são avaliados antes dos guards.

```kata
classify :: Int => Text
lambda x:
    > doubled 10: "grande"
    otherwise: "pequeno"
    with
        doubled := * x 2
```

* **Sintaxe:** `with` seguido de bindings indentados (`nome := expr`, sem keyword
  `let`). A ausência de `let` é visual — distingue o bloco `with` do corpo
  principal da cláusula.
* **Avaliação:** Os bindings são avaliados antes dos guards, em ordem top-down.
* **Escopo:** Bindings do `with` são visíveis em todos os guards da cláusula
  (não apenas nos que vêm depois — `with` é pós-escrito mas pré-avaliado).
* **Imutabilidade:** Os bindings são imutáveis (mesma semântica de `let`).
* **Representação na TAST:** Os bindings são preservados como
  `TypedWithBinding` na `TypedLambdaClause` (não desugared para `let` — o
  typeck infere cada binding e registra no escopo antes de processar os
  guards; o codegen os lowera antes do body da cláusula).

`with` também é usado para restrições de genéricos (placeholder — genéricos são
Fio 7; o parser reconhece, o typeck ignora as restrições em Fio 2).

## 17. Aridade Estrita e Aplicação Parcial (Currying)

* **Funções Puras:** Aridade fixa, conhecida estaticamente. Variádicas proibidas.
* **Actions:** Consomem obrigatoriamente uma tupla como argumento. `...` permite
  variadismo tipado no type checker.

### Currying Explícito com Hole (`_`)

`_` no lugar de um argumento congela a aplicação, gerando closure que aguarda o
argumento faltante. O desugar acontece no typeck: a TAST nunca contém `Hole` em
posição de argumento.

```kata
let soma_dez := + 10 _       # Int => Int
let subtrai_de := - _ 10     # Int => Int
let soma := + _ _            # Int Int => Int
```

### Pipeline (`|>`)

```kata
5 |> + 10 _              # 15 — Hole substituído por 5
5 |> + _ 10              # 15 — Hole em outra posição
5 |> + 1 _ |> * 2 _      # 12 — left-assoc: (5 + 1) * 2
5 |> double              # 10 — sem Hole, injeta como 1º argumento
```

### Closures com Captura Léxica

Na TAST, toda chamada de função é `TExpr::Closure`:

| Campo | Descrição |
|---|---|
| `name` | Nome da função para lookup do fn_ptr |
| `args` | Argumentos fornecidos (concretos + holes) |
| `holes` | Número de holes ainda não preenchidos |
| `captures` | Variáveis capturadas do escopo externo |
| `escapes` | Se a closure escapa → alocação heap/Arc |

### Modelo Stack/Arc e Escape Analysis

- **Stack (`CaptureStorage::Stack`):** Padrão. Captures na arena local, libertadas
  em O(1).
- **Heap / `Arc<T>` (`CaptureStorage::Heap`):** Se a closure escapa (retornada,
  enviada por canal, passada para `fork!`, armazenada em lista) — captures
  promovidas para heap global com `Arc<T>` nativo.

Escape Analysis em 4 passes (ver §2.2 pipeline).

### Chamada a Closures Escapadas (`FnValueCall`)

1. Carrega ponteiro da struct `Arc<ClosureBox>` do stack
2. Extrai `fn_ptr` e `captures`
3. Monta argumentos
4. Emite `call_indirect`

## 18. Otimização Avançada de Recursão e Despacho

Como o domínio funcional proíbe laços imperativos, a recursão é o único meio de
iteração. O compilador implementa passes de otimização para garantir que estas
abordagens não resultem em *Stack Overflow*.

### 18.1. TCO (Tail Call Optimization) — Delegado ao Cranelift

Iterações anteriores implementavam TCO próprio: um pass no `kata-optimizer`
que detectava `Call { func: self_name }` seguido de `Return` no IR e reescrevia
para `Jump`. Funcionava, mas operava por pattern matching no IR — não sabia se a
chamada era recursiva de cauda por design ou por acidente.

Em Kata-Lang, TCO é **delegado ao Cranelift 0.133**, que reconhece tail calls
nativamente e as converte em jumps. A TAST enriquecida carrega `tail_pos: bool`
em cada nó — informação que o typeck já computa mas que era descartada
antes do optimizer ver. Com essa anotação, o Cranelift tem a informação
semântica que precisa para otimizar corretamente.

O compilador Kata **não implementa TCO próprio**. Não há pass de TCO no
`kata-optimizer`. Se o Cranelift não otimizar um caso, a recursão consome stack
— mas não crasha (a não ser que exceda o limite da fiber).

### 18.2. TRMA (Tail Recursion Modulo Associativity) — Mantido no TAST

TRMA é mantido como pass do `kata-optimizer` no nível TAST. Diferente do TCO,
TRMA precisa de informação semântica que nenhum backend genérico tem: saber se
`+` é associativo (via `@associative(0)`) e qual o elemento neutro.

Quando `@associative(0)` anota o `+`, o otimizador interceta a recursão bloqueada
(`+ n (soma (- n 1))`), injeta um acumulador invisível e converte para recursão
de cauda perfeita. O Cranelift então aplica TCO no resultado.

TRMA só funciona com auto-recursão direta. Recursão mútua (A chama B que chama A)
não é otimizada — exigiria análise de SCC do call graph e fusão de múltiplas
funções num único bloco.

### 18.3. Stream Fusion

`map`/`filter`/`fold` marcados com `@builtin` geram nós TAST estruturados
(`Map { func, iterable }`, `Filter`, `Fold`). O optimizer fusiona nós aninhados
(`Map(Filter(arr))`) num único loop sem coleções intermediárias.

Esta otimização só é possível porque os nós TAST existem — se `map` fosse uma
chamada de função normal, o optimizer teria que reconstruir o padrão a partir
do CLIF, que é muito mais caro. O padrão `@builtin` (§19) é o que habilita esta
otimização.

## 19. Strings e Formatação (Dados Cegos)

Sem interpolação léxica. Strings são "dados cegos". A função `format` recebe
template literal + tupla de argumentos e substitui `{}` via FFI.

### Builtins Sintetizados

| Builtin | Síntese |
|---|---|
| `format` | Substitui `{}` via `kata_rt_text_replace_first` iterativo |
| `$` | Spread: interceptado pelo typeck, não builtin. `f $ (a, b)` → `f` recebe `a`, `b` como args separados. `TypedExprKind::Spread` (tipo `Unit`), expandido pelos handlers de aplicação. Nunca chega ao codegen. |
| `map` | `@builtin("map")` → nó TAST `Map { func, iterable }` |
| `filter` | `@builtin("filter")` → nó TAST `Filter { func, iterable }` |
| `fold` | `@builtin("fold")` → nó TAST `Fold { func, init, iterable }` |

Acesso posicional em tuplas (`t.0`, `t.1`) e `len` em tuplas não são builtins —
são special cases do typeck. `t.N` é `IndexAccess` com `Load` por offset;
`len tupla` é `IntLiteral` (element_types.len()). Ver §14.3.

`panic!` e `assert!` são Action builtins (não `@builtin`) — `panic!` é lowerado
direto para FFI (`kata_rt_panic`); `assert!` é interceptado no typeck e desugado
para `match cond { True: Unit, False: panic!(msg) }`. Ver §10.3.

Builtins nunca chegam ao backend como calls — o middle-end os converte para nós
TAST especializados antes do lowering.

## 20. Observabilidade, Logging e Pureza (`@log`)

O sistema de log do Kata é telemetria via canais CSP. Mensagens são publicadas
em **tópicos** (canais nomeados via registry thread_local) e consumidas via
`log_recv!()`. Não é um logger convencional com appenders e formatadores — é
pub/sub sobre a infraestrutura de canais existente.

### 20.1. Diretiva `@log`

Anotação em actions e funções nomeadas que injeta `kata_rt_log_publish` no
prólogo (`when: "enter"`) ou epílogo (`when: "exit"`) da definição. Permite
emitir telemetria estruturada sem contaminar a assinatura matemática — a pureza
nominal da função não muda.

```kata
@log{msg: "processando {x}, resultado: {r}", when: "exit", level: LogLevel::Info, topic: "audit", policy: "block"}
action processar (x::Int) -> Int
  let r := * x 2
  r
```

**Campos:**

| Campo | Tipo | Obrigatório | Descrição |
|---|---|---|---|
| `msg` | `Text` | sim | Template compile-time. `{expr}` interpola expressão do escopo (Ident ou `Ident.field`). `{{` escapa `{` literal; `}}` escapa `}`. Desugara para `format "template" (expr1, ...)` via `infer_format`. |
| `when` | `Text` | sim | `"enter"` = loga no prólogo. `"exit"` = loga no epílogo. |
| `level` | `LogLevel` | não | Variante do enum `LogLevel` (`Debug`/`Info`/`Warn`/`Error`). Default: `Info`. |
| `topic` | `Text` | não | Nome do canal. Default: herdado do fiber ancestral (ou `"default"`). |
| `policy` | `Text` | não | `"drop"` (Broadcast, fire-and-forget) ou `"block"` (Queue cap=1, backpressure). Default: herdado (ou `"drop"`). |

**Restrições de `when`:**
- `when: "enter"` → placeholders do `msg` só podem referenciar **parâmetros**
  da função. Referenciar variável do corpo é erro compile-time.
- `when: "exit"` → placeholders podem referenciar params e variáveis do corpo.
  O codegen injeta a publicação no epílogo (antes do retorno).

### 20.2. Action nativa `log!()`

Publicação explícita no corpo de actions. Dispara na execução da linha (não no
wrapping como `@log`). A mensagem é **dinâmica** (construída em runtime) — não
há interpolação de template como no `@log`.

```kata
log!(LogLevel::Info, "mensagem", "audit", "drop")
```

| Pos | Tipo | Descrição |
|---|---|---|
| 0 | `LogLevel` | Level da mensagem. |
| 1 | `Text` | Mensagem dinâmica (sem interpolação `{expr}`). |
| 2 | `Text` | Tópico. Opcional — default herdado ou `"default"`. |
| 3 | `Text` | Policy. Opcional — default herdado ou `"drop"`. |

Typeck aceita 2, 3 ou 4 args.

### 20.3. Action nativa `log_recv!()`

Recebe a próxima mensagem de telemetria do tópico. Bloqueia (yield point via
`BlockedOnRecv`) até chegar mensagem. Retorna `Text` (payload) ou `Unit` se o
canal fechou. Precisa estar em fiber context.

```kata
let msg := log_recv!("audit")
```

Para tópicos Broadcast (policy `"drop"`), o receiver é criado eagerly no
`get_or_create_topic` e cached em `RECEIVER_REGISTRY` (thread_local).

### 20.4. Action nativa `log_config!()`

Configura defaults de `topic`/`policy`/`level` para o fiber atual e
descendentes. Herdado via snapshot no `kata_rt_spawn` (copia `LOG_CONFIG` TLS
do pai para o filho).

```kata
log_config!("audit", "block", LogLevel::Info)
```

### 20.5. Canais e policies

Tópicos são canais nomeados, resolvidos sob demanda num registry
`HashMap<String, i64>` (nome → handle).

- **`"drop"`** → canal Broadcast. Fire-and-forget — não bloqueia o publisher.
  Cada receiver mantém seu próprio `last_seen_version`; receivers lentos perdem
  mensagens intermediárias (o `BroadcastInner` guarda só a última versão).
- **`"block"`** → Queue bounded (cap=1). Backpressure — bloqueia o publisher
  via `WaitingOnChannelSend` até o consumidor liberar o slot com `channel_recv`.

### 20.6. Enum `LogLevel`

```kata
enum LogLevel
  Debug
  Info
  Warn
  Error
```

Fixo no `stdlib/core.kata`. Tags: `Debug=0, Info=1, Warn=2, Error=3`. Validado
em compile-time. **O runtime atualmente ignora o level** — o parâmetro é
passado na FFI mas não há filtragem. Reservado para filtragem futura.

### 20.7. Pureza

`@log` não muda a assinatura. O codegen insere o efeito colateral
invisivelmente — na semântica da linguagem, a pureza não muda; no máximo a
resposta é adiada (com `policy: "block"`).

## 21. Interoperabilidade e Baixo Nível (FFI)

`@ffi("nome_simbolo_c")` informa que o corpo é importado de biblioteca externa.
O compilador confia na assinatura fornecida — se a função C tiver efeitos
colaterais e for importada como Função pura, o otimizador pode extirpá-la ou
paralelizar indevidamente. A responsabilidade do isolamento recai sobre o
desenvolvedor da biblioteca.

## 22. Operações Primitivas e a Interface de Risco

A biblioteca padrão estabelece uma distinção rigorosa entre operações
garantidas matematicamente e operações dinâmicas passíveis de falha. O
compilador não mascara falhas com retornos silenciosos; ele as tipifica.

### 22.1. Divisão: Exatidão Estática vs Resolução Dinâmica

* **`/` (Exato):** Exige divisor `NonZero` (refined). Sendo o zero
  matematicamente impossível por contrato, o retorno é o valor puro direto.
* **`div` (Dinâmica):** Aceita `NUM` normais, retorna `Result::(NUM, Err)`.

### 22.2. Acesso Posicional: Sintaxe Uniforme, Retorno Distinto

O mesmo princípio aplica-se ao acesso posicional (ver §14.3):

* **Tupla `.N` (Exato):** O typeck conhece o tamanho estaticamente. Bounds
  check em compile-time. Retorno direto — `t.0` é `Int`, não `Result`.
* **Coleção `.N` / `at` (Dinâmico):** O tamanho é runtime. `.N` em coleções é
  syntactic sugar para `at` (interface `INDEXABLE(A)`), que retorna
  `Result::(A, Err)`. O programador usa `?` (em Actions) ou `match` explícito
  para desempacotar.

A distinção é a mesma: prova compile-time → sem `Result`; risco runtime →
`Result` obrigatório.

### 22.3. Extração Fatal (`unwrap_or_panic!`)

No domínio impuro das Actions, o compilador fornece `unwrap_or_panic!(Result
"Mensagem")`. Desempacota o valor em caso de sucesso ou aciona `panic!`,
registrando o rastro no escalonador.

### 22.4. Socket I/O

`Socket` é um tipo opaco intrínseco (`Ty::Socket`) — handle para socket
TCP ou Unix domain aberto. O usuário não enxerga fields nem constrói
`Socket` diretamente; o único modo de obter um é via `open!` ou `listen!`.

**Enums do prelude:**

```kata
enum SocketKind
    TCP(Text)      # payload = endereço "host:port"
    Unix(Text)     # payload = path do socket file

enum SocketMode
    Listener       # open! → bind + listen → socket passivo
    Connected      # open! → connect → socket ativo (full-duplex)
```

**Actions de socket no prelude:**

```kata
open (kind::SocketKind, mode::SocketMode) => Result::(Socket, Text)
listen (listener::Socket) => Result::(Socket, Text)
read (s::Socket) => Result::(Bytes, Text)
read (s::Socket, n::Int) => Result::(Bytes, Text)
readline (s::Socket) => Result::(Text, Text)
write (s::Socket, content::Text) => Result::(Unit, Text)
write (s::Socket, content::Bytes) => Result::(Unit, Text)
close (s::Socket) => Unit
```

- **`open!` despacha por kind × mode** (4 paths: TCP listener, TCP
  connected, Unix listener, Unix connected).
- **`listen!` opera sobre listener** — retorna socket `Connected` do
  cliente aceito. O listener continua passivo.
- **`read!` tem 2 overloads por aridade** — `read(s)` (slurp) e
  `read(s, n)` (chunk de até n bytes). Mesma convenção de File.
- **`readline!` lê uma linha (até `\n`)** — usa buffer parcial persistente
  em `SocketInner.line_buf`. TCP não preserva fronteiras de mensagem, então
  uma linha pode chegar em múltiplos chunks. Não misturar `readline!` com
  `read!`/`read!(s, n)` no mesmo socket — estas lêem do FD diretamente,
  ignorando o buffer. Mesma separação que Go `bufio` / Rust `BufReader`.
- **`write!` tem 2 overloads por tipo** — `Text` (C string) e `Bytes`
  (suporta null bytes). FFIs separadas.
- **Non-blocking obrigatório** — todo socket é `O_NONBLOCK`. O scheduler
  cooperativo gerencia o bloqueio via `poll` + suspensão de fiber.
- **`SO_REUSEADDR` hardcoded** em listeners TCP.
- **EOF em sockets → `Err("EOF")`** — consistente com File.
- **Close no epílogo** — `io_handle_vars` rastreia sockets abertos;
  o epílogo despacha `kata_rt_socket_close` por `IoHandleKind::Socket`.

**Distinção Listener vs Connected:**

| Operação | `Listener` | `Connected` |
|---|---|---|
| `listen!` (aceitar) | ✅ | ❌ `Err` |
| `read!` | ❌ `Err` | ✅ |
| `write!` | ❌ `Err` | ✅ |

## 23. Orquestração Não-Determinística (`select`)

`select` multiplexa operações de canais, files e sockets. A Action cede ao
scheduler e é acordada quando um evento se concretiza. Os canais são
verificados **em ordem** (primeiro pronto é selecionado) — não há
aleatoriedade. `timeout N` (ms) como válvula de escape.

Braços de I/O (`read(handle, n) <! binding: body`) aceitam `File` ou
`Socket` como handle. O codegen separa braços por tipo em compile-time
(`channel_arms`, `file_arms`, `socket_arms`) e passa arrays separados
para `kata_rt_select_combined` (7 args: chan_ptr, n_c, file_ptr, n_f,
socket_ptr, n_s, timeout_ms). Sockets bloqueiam cooperativamente via
`poll(POLLIN)` — diferente de arquivos regulares (sempre prontos).

## 24. Anatomia do Runtime (`kata-rt`) e a Ponte C-ABI

O binário final encapsula código de máquina (Cranelift) acoplado ao runtime
(`kata-rt`).

* **Scheduler single-threaded cooperativo:** M fibers (tasks Kata) em 1 thread
  OS, com round-robin e yield cooperativo. TLS para suspend/resume. O scheduler
  é uma struct explícita. Multithread (M:N com work-stealing) é aspiracional.
* **Corrotinas Stackful (wasmtime-fiber):** Cada Action é encapsulada numa
  corrotina nativa. `<!` bloqueante faz yield da fiber, não da thread OS.
* **ARC manual (CaptureBox):** Reference counting gerenciado pelo codegen via
  FFI (`kata_rt_incref`/`kata_rt_decref`). CaptureBox alocado na root arena
  (TrackedArena); quando refcount chega a 0, o bloco é liberado individualmente.
  Refcount não-atomic (single-threaded).
* **`spawn!` multiprocess:** Fork de processo OS com IPC. Isolamento total
  para CPU-bound pesado. Valores são serializados por marshalling (by-value).

## 25. Diagnostics

O `kata-diagnostics` é um único crate com 3 submódulos (frontend, middleend,
backend), sem códigos numéricos. Erros usam códigos namespaced por domínio:

```rust
#[error("tipo incompatível: esperado `{expected}`, encontrado `{found}`")]
#[diagnostic(code = "type.mismatch")]
TypeMismatch { expected: String, found: String, #[label] span: Span }
```

Isso dá mensagens imediatamente compreensíveis ao desenvolvedor (sem prefixo
numérico) e códigos estáveis para ferramentas (LSP, CI, docs) sem renumeração
quando novos erros são adicionados.

### Por que 1 crate com 3 submódulos (não 2 crates)

Iterações anteriores tinham 2 crates: `kata-diagnostics` (frontend) e
`kata-diagnostics-backend` (backend). A separação causava três problemas:

1. **Códigos de erro colidindo**: `E801` significava coisas diferentes em cada
   crate (`TryPropagateInPureContext` no frontend vs `NotImplemented` no backend).
   O usuário não sabia qual `E801` estava vendo.
2. **Crates dependendo de ambos**: 3 crates (typeck, module-loader, codegen)
   dependiam dos dois crates de diagnostics simultaneamente — o firewall de
   compilação que a separação deveria proporcionar não existia para eles.
3. **Erros no crate errado**: `ModuleLoaderError` estava no backend mas era
   consumido no frontend.

A fusão em 1 crate com 3 submódulos (frontend, middleend, backend) resolve os
três: códigos namespaced não colidem, ninguém depende de dois crates, e erros
ficam no submódulo da fase correta.

### Por que sem códigos numéricos

Códigos como `E103` são ruído cognitivo — o desenvolvedor precisa cruzar o número
com um catálogo para entender o que aconteceu, e o catálogo é outra coisa para
manter sincronizada. Quando você adiciona um erro novo, precisa decidir se é
`E103b` ou `E114` ou `E605` — renumeração artificial.

Códigos namespaced (`type.mismatch`, `parse.unexpected_token`, `codegen.internal`)
são auto-documentáveis e estáveis. Adicionar `type.generic_conflict` não exige
renumerar nada — o namespace já determina.

## 26. REPL Interativo (`kata repl`)

O REPL permite exploração incremental da linguagem com estado persistente entre
expressões. O design combina um `TypeEnv` que acumula bindings com um `JITModule`
fresco por avaliação — o Cranelift não suporta extensão após `finalize_definitions`,
então cada expressão recompila tudo (prelude + items acumulados + nova expressão).

O custo é aceitável: o compile-time de uma expressão via Cranelift JIT é
milissegundos, sem percepção de latência para uso interativo.

### 26.1. Arquitetura

```
┌─────────────────────────────────────────────────┐
│ REPL Session                                     │
│                                                  │
│  items: Vec<Spanned<Item>> (persistente)         │
│  ├── let bindings, sigs, data, enum, implements  │
│  ├── cada item é re-processado a cada expressão  │
│  └── persistência estrutural, não mutação de env │
│                                                  │
│  prelude: ResolvedModule (recarregado em :reset) │
│                                                  │
│  Loop:                                           │
│    1. Ler input (rustyline)                      │
│    2. Se comando (:), executar                   │
│    3. Se expressão:                               │
│       a. lex → parse                              │
│       b. merge items acumulados + novo input      │
│       c. resolve → infer_module                   │
│       d. monomorphize → optimize → tree_shake     │
│       e. comptime pass                            │
│       f. JITModule fresco → jit_eval → display    │
│       g. Sucesso: item fica na lista             │
│       h. Erro: rollback (item removido)           │
└─────────────────────────────────────────────────┘
```

A persistência é estrutural: items top-level (`let`, `Sig`, `data`, `enum`,
`implements`, `interface`) são acumulados numa `Vec<Spanned<Item>>` e
re-processados a cada nova expressão. O `TypeEnv` não é mutado diretamente —
a próxima avaliação vê todos os items anteriores porque eles estão na lista.

### 26.2. Comandos

| Comando | Descrição |
|---|---|
| `:help` | Lista comandos disponíveis |
| `:type <expr>` | Infere e mostra o tipo de `<expr>` sem executar |
| `:env` | Mostra bindings e tipos atuais |
| `:load <file>` | Carrega arquivo `.kata` — items entram na sessão |
| `:reset` | Limpa bindings, recarrega prelude |
| `:quit` | Sai do REPL (`:exit` também funciona) |

### 26.3. Multiline

O REPL detecta automaticamente quando uma expressão precisa de múltiplas
linhas. A heurística combina três estratégias:

**1. Assinatura de função (`Sig` + `lambda`):** Se a primeira linha contém `::`
e `=>` (sem `@ffi`), o modo multiline ativa. Cláusulas `lambda` seguintes
podem estar no mesmo nível (não-indentadas). Uma linha em branco encerra o bloco.

```
kata> fat :: Int Int => Int
   ... lambda 0 acc: acc
   ... lambda n acc: fat (- n 1) (* n acc)
   ...
kata> fat 5 1
120
```

**2. Action:** Se a primeira linha termina com `=>`, o body indentado segue.

```
kata> action ola => Text
   ...     "hello"
   ...
kata> ola!()
hello
```

**3. Blocos indentados (`match`, `enum`, `interface`, `implements`):** Se a
primeira linha começa com uma destas palavras-chave, o modo multiline ativa.
Linhas indentadas (começam com espaço ou tab) são acumuladas. Uma linha
não-indentada encerra o bloco.

```
kata> match = 1 1
   ...     True: "igual"
   ...     False: "diferente"
   ...
kata>
igual
```

```
kata> enum Cor
   ...     Vermelho
   ...     Verde
   ...     Azul
   ...
kata> let c := Cor::Verde
()
kata> :type c
Cor
```

**Fallback:** Se nenhuma heurística dispara, o REPL tenta parsear o input. Se
o parser falha com `<EOF>` (input incompleto), continua lendo linhas.

### 26.4. `:load <file>`

Carrega um arquivo `.kata` e processa todos os items como se fossem digitados
no REPL. `let` bindings, `data`, `enum`, `Sig`+`lambda`, `implements` entram
na sessão. Se o arquivo contém `EntryExpr`, executa e mostra o resultado.

```
kata> :load examples/fatorial.kata
120
kata> fat 6 1
720
```

Se o arquivo só tem declarações (sem `EntryExpr`), os items são adicionados sem
execução:

```
kata> :load examples/modules/mock_math.kata
carregado: examples/modules/mock_math.kata
kata> mock_math.dobrar 21
42
```

### 26.5. `:type <expr>`

Executa o pipeline até `infer_module` e imprime o tipo do entry point sem
fazer codegen. Útil para inspecionar tipos sem side-effects.

```
kata> :type + 1 2
Int
kata> :type (1, 2)
(Int, Int)
kata> let f := + 10 _
kata> :type f
Int -> Int
```

### 26.6. `:env`

Mostra todos os bindings atuais com seus tipos. Roda o pipeline até
`TypedModule` para obter os tipos inferidos.

```
kata> let x := 10
kata> let y := 20.0
kata> :env
  x: Int
  y: Float
```

### 26.7. `:reset`

Limpa todos os bindings do usuário e recarrega o prelude. Equivalente a sair
e reentrar no REPL.

```
kata> let x := 42
kata> :env
  x: Int
kata> :reset
sessão resetada — prelude recarregado
kata> :env
(nenhum binding)
kata> + 1 2
3
```

### 26.8. Tratamento de Erros

Erros de parse, tipo, ou runtime **não abortam a sessão**. O item que falhou é
removido da lista (rollback) e o usuário pode corrigir e reintentar. O estado
anterior é preservado.

```
kata> let x := 10
kata> undefined_name
erro de tipo: UnboundName { name: "undefined_name", ... }
kata> :env
  x: Int
kata> + x 5
15
```

### 26.9. Histórico

O histórico de inputs é persistido em `~/.kata_repl_history` via rustyline.
Setas ↑/↓ navegam o histórico. O histórico é salvo ao sair do REPL (`:quit`
ou Ctrl-D).

---

## 27. Reflexão de Funções

Kata5 permite inspecionar metadata de funções e actions em tempo de execução
ou compilação através de acesso de campo (`.`) sobre valores funcionais. Isto
é **reflexão estruturada** — não é introspecção arbitrária de runtime, mas
um conjunto fixo de fields com semântica definida pelo typeck.

A reflexão é o pré-requisito para o sistema de diretivas definidas pelo
usuário (decorators), onde `f.name` e `f.arity` são passados como metadata
para hooks de before/after.

### 27.1. Fields Disponíveis

| Field | Tipo (estático, elemento de lista) | Tipo (dinâmico/desambiguado) | Descrição |
|---|---|---|---|
| `name` | `Text` | `Text` | Nome no DispatchTable (ou nome do binding para lambdas) |
| `arity` | `Int` | `Int` | Número de parâmetros |
| `param_types` | `List::Text` | `List::Text` | Tipos dos parâmetros como texto |
| `return_type` | `Text` | `Text` | Tipo de retorno como texto |
| `is_action` | `Boolean` | `Boolean` | `True` se Action, `False` se função pura |

Field desconhecido (`f.foo`) → erro de compilação. Receptor não-funcional
(`42.name`) → erro de compilação.

### 27.2. Os Quatro Casos de Dispatch

O typeck distingue quatro casos baseado no receptor e no tipo do index. A
distinção é **estática** — resolvida em compile-time, sem dispatch em
runtime sobre o tipo do receptor.

#### Caso 1: Estático sem desambiguação — sempre lista

```kata
soma :: Int Int => Int
lambda a b: + a b

soma.name          # → ["soma"]
soma.arity         # → [2]
soma.param_types   # → [["Int", "Int"]]
soma.return_type   # → ["Int"]
soma.is_action     # → [False]
```

Quando o receptor é `Ident` direto para uma função nomeada no
`DispatchTable`, o typeck consulta todas as overloads e produz `ListLit` —
um elemento por overload. O tipo do elemento **não muda** quando overloads
são adicionadas: `soma.arity` sempre é `List::Int`, nunca `Int`.

Actions também seguem este caso (4a). Como actions não são first-class, a
reflexão de actions é sempre estática via DispatchTable — sempre lista.

#### Caso 2: Estático desambiguado — escalar

```kata
soma.(Int Int).arity   # → 2
soma.(Int Int).name    # → "soma"
```

A sintaxe `f.(T1 T2 ...)` seleciona uma overload específica por tipos de
parâmetros (ver §27.3). A partir daí, `.field` é escalar — um único valor,
não lista.

#### Caso 3: Dinâmico — escalar via sidecar table

```kata
let g := soma
g.name    # → "soma"   (escalar)
g.arity   # → 2        (escalar)
```

Quando o receptor é uma variável local (`let g := soma`) com `fn_alias`
rastreando a função original, o typeck não sabe em compile-time qual overload
está em `g` (pode haver múltiplas). Emite chamada FFI para
`kata_rt_fn_meta_lookup(fn_ptr, field_id)`, que faz binary search O(log N)
na sidecar table (`__kata_fn_meta_table`) em runtime.

#### Caso 4: Lambda com binding — lista de length 1

```kata
let h := lambda x: + x 1
h.name    # → ["h"]   (lista, length 1)
```

Quando o receptor é uma variável local com `Ty::Function` mas **sem**
`fn_alias` (não é alias para função nomeada) e não está no DispatchTable, é
uma lambda anônima. O typeck produz `ListLit` com 1 elemento usando o nome
do binding.

### 27.3. Desambiguação por Tipos: `f.(Int Int)`

```kata
soma :: Int Int => Int
lambda a b: + a b
soma :: Text Text => Text
lambda a b: a

soma.(Int Int).arity        # → 2 (escalar — overload Int Int)
soma.(Text Text).name       # → "soma" (escalar — overload Text Text)
soma.(Float Float).arity    # → erro: nenhuma overload compatível
```

**Sintaxe.** `f.(T1 T2 ...)` onde cada `Ti` é um `TypeExpr` resolvido pelo
typeck para `Ty`. Tipos separados por espaço. Parênteses obrigatórios.

**Parser.** No loop de DotAccess, `.(...)` pode conter `IntLit` (→
`DotIndex::Int`, indexação de tupla) ou `TypeExpr` (→ `DotIndex::Type`,
desambiguação). O lexer distingue naturalmente: `Token::IntLit` produz
inteiros; `Token::Ident("Int")` produz tipos. O parser lê TypeExprs
separados por espaço até encontrar `)`.

**Typeck.** Filtra overloads (não-actions) por `params == requested`.
- 1 match → `TypedExprKind::Ident` com `Ty::Function(params, ret)`
  específica. A partir daí, `.field` usa a overload selecionada (escalar).
- 0 matches → erro `TypeMismatch` ("nenhuma overload compatível").
- 2+ matches (mesmos params, ret diferente) → erro ("múltiplas overloads com
  mesmos params — ambígua").

**Codegen.** Não precisa de mudança. O typeck produz um `Ident` com
`Ty::Function` concreta, e o codegen já resolve via `symbol_table.get((name,
params, ret))` → `FuncId` exata.

### 27.4. Provenance Tracking (`fn_alias`)

A distinção entre caso 3 (dinâmico, escalar) e caso 4 (lambda, lista) exige
saber se um binding `let g := ...` é alias para função nomeada ou lambda
anônima. Isto é rastreado via `fn_alias: Option<String>` em `TypeBinding`.

No `Expr::Let`, se o value é `Expr::Ident` apontando para função nomeada no
`DispatchTable`, o binding recebe `fn_alias = Some("nome_original")`.
Lambdas (`let g := lambda...`) recebem `fn_alias = None`.

A regra de dispatch no `dot_access`:

| Condição | Caso | Retorno |
|---|---|---|
| Nome no DispatchTable (Ident direto) | 1 — estático | Lista |
| `fn_alias = Some`, nome no DispatchTable | 3 — dinâmico | Escalar (sidecar table) |
| `fn_alias = None`, não no DispatchTable | 4 — lambda | Lista (length 1) |

### 27.5. Sidecar Table e ABI do Caso Dinâmico

O caso dinâmico não consulta o DispatchTable em runtime (ele é destruído após
o typeck). Em vez disso, o codegen emite um data symbol
`__kata_fn_meta_table` com entries de 56 bytes cada:

```
offset  0: fn_ptr           (i64 — relocation resolvida pelo JIT)
offset  8: name_ptr         (i64 — ponteiro para string estática)
offset 16: arity            (i64 — número de parâmetros)
offset 24: param_types_ptr  (i64 — ponteiro para array de string ptrs)
offset 32: param_types_len  (i64 — número de param types)
offset 40: return_type_ptr  (i64 — ponteiro para string estática)
offset 48: is_action        (i64 — 0 = Function, 1 = Action)
```

O runtime registra esta tabela no prólogo do entry point (antes da execução
do código do usuário) e consulta via binary search ordenado por `fn_ptr`.

**ABI do lookup.** `kata_rt_fn_meta_lookup(fn_ptr, field_id)` retorna:
- `arity`: SMI-tagged `(val << 1) | 1` via `kata_rt_tag_int`
- `param_types`: List Cons na root arena via `kata_rt_list_cons`
- `name`/`return_type`: C string ptr
- `is_action`: inline 0/1

**CaptureBox.** `let g := soma` produz um `CaptureBox` (box_ptr), não
`fn_ptr` direto. O codegen intercepta `kata_rt_fn_meta_lookup` em
`closure.rs` e faz `load(call_args[0], 0)` para extrair `fn_ptr` do
CaptureBox antes de passar para a FFI.

**SMI untag.** O `field_id` produzido pelo typeck como `IntLit` sofre SMI
tagging no codegen (`encode_smi(0) = 1`). A interceptação faz untag
(`(smi - 1) >> 1`) antes de passar para a FFI.

### 27.6. Regras e Edge Cases

1. **Actions não são first-class.** Reflexão de actions é sempre estática
   via DispatchTable (caso 1 — sempre lista). `let g := action_name` não
   existe.
2. **Sempre lista no caso estático sem desambiguação.** `f.arity` →
   `List::Int`. `f.(Int Int).arity` → `Int` escalar (desambiguado).
3. **Caso dinâmico sempre escalar.** `fn_ptr` identifica overload exata na
   sidecar table.
4. **ABI não muda.** Função continua `I64` na ABI. `ty_to_clif` não muda.
5. **`display()` de `Ty::Function`** produz `Lambda(Int Int -> Int)` —
   renomeado de `Function` para `Lambda` na linguagem para evitar confusão
   com `Ty::Action`.
6. **Overloads com mesmos params, ret diferente.** `soma.(Int Int)` com duas
   overloads `Int Int => Int` e `Int Int => Text` → erro de ambiguidade.
7. **Field desconhecido** (`f.foo`) → erro de compilação
   (`is_reflection_field` retorna `false`).
8. **Receptor não-funcional** (`42.name`) → erro de compilação
   (`NotIndexable`).