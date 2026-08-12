# PRD — Kata Book: Guia Introdutório da Linguagem

**Status:** Rascunho
**Data:** 2026-08-10
**Público-alvo:** Programadores interessados em linguagens funcionais, sem
experiência prévia com Kata. Material para mostrar a amigos/coleagues.
**Formato:** Texto plano (markdown), um único arquivo, distribuído junto com o
binário ou como arquivo solto.

## 1. Objetivo

Escrever um guia introdutório estilo "The Rust Book" que apresente Kata desde
o primeiro "hello world" até recursos avançados (pattern matching, CSP, tipos
refinados), em texto plano legível em qualquer terminal. O guia deve ser:

- **Autocontido** — não exige conhecimento de Rust, Cranelift, ou internals
- **Baseado em código real** — todo exemplo deve compilar e executar com o
  binário `kata` atual. Validação: `kata run <exemplo>` ou `kata repl`
- **Progressivo** — cada capítulo constrói sobre o anterior, sem saltar
- **Honesto** — não documenta features aspiracionais. Se algo não funciona
  hoje, não está no livro

## 2. Estrutura

### Capítulos obrigatórios (fase 1)

1. **Olá, Kata** — instalar, primeiro programa, `kata run`, `kata repl`
2. **Sintaxe básica** — notação prefixa, literais, comentários, `echo!`
3. **Bindings e tipos** — `let`, tipos primitivos (Int, Float, Text, Boolean,
   Unit), anotação `::`, shadowing
4. **Funções** — assinaturas `nome :: T1 T2 => Ret`, `lambda`, múltiplas
   cláusulas, recursão, TCO/TRMA
5. **Pattern matching** — `match`, variantes qualificadas/desqualificadas,
   guards, `otherwise`, `with`
6. **Actions** — barreira pure/impure, `action`, `!`, `echo!`, `var`, `loop`,
   `break`/`continue`, `;` vs retorno implícito
7. **Colecões** — List `[1 2 3]`, `cons [h : t]`, tupla `(a, b)`, Dict
   `{k: v}`, Set `{|1 2 3|}`, ranges `[1..1..10]`, `for x in`
8. **HOFs e closures** — `map`, `filter`, `fold`, holes `_`, currying, capture,
   `|>` pipeline
9. **Enums e structs** — `enum`, variantes (unitárias, payload, constantes,
   predicadas), `data`, fields, dot access
10. **Actions avançadas** — `fork!`, `spawn!`, channels `<!`/`!>`, `select`,
    `timeout`, `queue!`, `broadcast!`, `sleep!`

### Capítulos opcionais (fase 2 — se tempo permitir)

11. **Tipos refinados** — `data (Int, > _ 0) as PositiveInt`, smart
    constructors, `refines`, `?`, `|`, downcast `::`
12. **Módulos** — `import`, `export`, selective import
13. **Otimizações** — TCO, TRMA, stream fusion, `@comptime`, `@cache`
14. **REPL interativo** — `:type`, `:env`, `:load`, `:reset`, multiline,
    persistência de bindings

## 3. Princípios de escrita

### 3.1. Cada exemplo é executável

Todo bloco de código deve funcionar com `kata run` ou `kata repl`. Antes de
incluir no livro, validar:

```bash
echo '<exemplo>' > /tmp/test.kata && kata run /tmp/test.kata
# ou
printf '<linha1>\n<l linha2>\n:quit\n' | kata repl
```

### 3.2. Notação prefixa desde o início

O livro não pede desculpas pela notação prefixa — apresenta como natural desde
o capítulo 1. `+ 1 2` é o primeiro exemplo, sem comparar com infix.

### 3.3. Sem `if`

O livro nunca menciona `if`. Condicionais são introducidas como pattern
matching + guards no capítulo 5, antes de qualquer necessidade de branching.
O leitor aprende `match` como a forma natural de condicional.

### 3.4. Barreira pure/impure introduzida cedo

O capítulo 6 (Actions) estabelece a barreira: funções puras não têm `!`,
actions têm. Isto é apresentado como feature, não como limitação. O porquê
(garantias de otimização, ausência de efeitos colaterais) é explicado
brevemente.

### 3.5. Progressão de complexidade

- Caps 1-4: só funções puras e literais
- Cap 5: pattern matching (ainda puro)
- Cap 6: actions (primeira impureza)
- Caps 7-8: coleções e HOFs (puro, mas usa tipos compostos)
- Cap 9: tipos de dados (puro)
- Cap 10: CSP (impuro, actions)

### 3.6. Linguagem acessível

- Português do Brasil
- Sem jargão de compilador (sem "TAST", "CLIF", "Cranelift", "monomorphização")
- Termos técnicos da linguagem (action, binding, guard, hole) são definidos
  no momento de introdução
- Tom direto, não acadêmico

## 4. Esboço por capítulo

### Cap 1 — Olá, Kata

```
# Instalação
kata --version

# Primeiro programa
echo '+ 1 2' > hello.kata
kata run hello.kata
# output: 3

# REPL
kata repl
kata> + 1 2
3
kata> :quit
```

### Cap 2 — Sintaxe básica

- Notação prefixa: `+ 1 2`, `* 3 4`, `- 10 3`
- Comentários: `#`
- Literais: Int (`42`, `0xFF`, `1_000`), Float (`3.14`), Text (`"hello"`),
  Boolean (`True`, `False`), Unit (`()`)
- `echo!` para imprimir
- `show` para converter qualquer valor em Text

### Cap 3 — Bindings e tipos

- `let x := 42` — binding imutável
- Shadowing: `let x := 42` → `let x := 99`
- Tipos: `Int`, `Float`, `Text`, `Boolean`, `Unit`
- Anotação: `let x := 42 :: Int` (redundante aqui, útil em ambiguidade)
- Rationals: `3.14::Rational` (precisão exata)
- BigInt: `* 99999999999999999999 99999999999999999999`

### Cap 4 — Funções

- Assinatura: `dobrar :: Int => Int`
- Corpo: `lambda x: * x 2`
- Múltiplas cláusulas: `fat :: Int Int => Int` / `lambda 0 acc: acc` /
  `lambda n acc: fat (- n 1) (* n acc)`
- Recursão: fatorial, fibonacci
- TCO: `fat 1000000 1` não estoura stack (tail-recursivo)
- TRMA: `soma 1000000` não estoura stack (optimizer reescreve)

### Cap 5 — Pattern matching

- `match True` / `match Optional::Some(42)`
- Variantes desqualificadas: `Ok v:` / `Err e:`
- Guards: `> x 0: x` / `otherwise: - 0 x`
- `with` block: bindings prévios visíveis nos guards
- Exaustividade: o compilador verifica

### Cap 6 — Actions

- `action greet` / `echo!("hello")` / `greet!()`
- Barreira: funções puras vs actions
- `var` — binding mutável (só em actions)
- `loop` / `break` / `continue`
- `;` — terminador de statement vs retorno implícito
- `let x := 5; echo!(x)` — dois statements na mesma linha

### Cap 7 — Coleções

- List: `[1 2 3]`, `[h : t]` (cons), `[]` (vazia)
- Tupla: `(1, "hello", True)`
- Dict: `{"nome": "Ana" "idade": 30}`
- Set: `{|1 2 3|}`
- Range: `[1..1..10]`, `[0..2..=20]`, `[10..-1..=0]`
- `for x in lista` — iteração
- `+` para concat de listas, `in` para contains

### Cap 8 — HOFs e closures

- `map (* _ 2) [1 2 3]` — hole cria closure
- `filter (lambda x: > x 0) [1 -2 3]`
- `fold (+) 0 [1 2 3]`
- `|>` — pipeline: `5 |> + 1 _ |> * 2 _`
- Capture: `let n := 10` / `let add10 := + _ n` / `add10 5`

### Cap 9 — Enums e structs

- `enum Cor` / `Verde` / `Amarelo` / `Vermelho`
- Variante com payload: `Optional::Some(42)`
- Smart constructor: `IMC(22.5)` — valida predicado
- `data Pessoa (nome::Text idade::Int)`
- Dot access: `p.nome`
- `match` em enums com payload

### Cap 10 — Actions avançadas (CSP)

- `fork!(worker, (args))` — spawn de fiber
- Channels: `<!` (send), `!>` (receive)
- `queue!(capacidade)` — channel bufferizado
- `select` / `timeout ms`
- `broadcast!()` / `subscribe!()`
- `sleep!(ms)` — yield cooperativo
- Concorrência cooperativa (não preemptiva)

## 5. Validação

Antes de publicar:

1. **Todo exemplo executa** — `kata run` ou `kata repl` em cada bloco de código
2. **Sem features aspiracionais** — nada que não compile hoje
3. **Progressão coerente** — cada capítulo só usa conceitos introduzidos
   anteriormente
4. **Sem jargão de compilador** — revisar por leitor não-compiler-person

## 6. Formato de entrega

- Arquivo único: `docs/kata-book.md` (ou `KATA_BOOK.md` na raiz)
- ~2000-4000 palavras (não é exaustivo, é introdutório)
- Markdown plano — legível em terminal, renderizável em GitHub
- Blocos de código marcados com ```kata
- Sem diagramas (texto plano)

## 7. Fonte de verdade

- **Sintaxe:** `docs/sintaxe-mapa.md` + exemplos em `examples/`
- **Comportamento:** binário `kata` atual (release build)
- **Manual técnico:** `docs/Kata-lang-manual.md` (referência, não fonte primária
  — o livro é mais acessível que o manual)
- **O que existe vs aspiracional:** validar cada claim contra `kata run` ou
  `kata repl`

## 8. O que o livro NÃO é

- Não é o manual técnico (não cobre pipeline de compilação, ABI, FFI,
  Cranelift, arenas, ARC)
- Não é referência completa (não lista todos os FFIs, todas as diretivas,
  todos os variants de erro)
- Não é tutorial de implementação (não explica como o compilador funciona
  internamente)
- Não cobre LSP, AOT build, TextMate grammar