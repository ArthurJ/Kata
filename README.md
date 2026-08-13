# Kata Language

Kata é uma linguagem de programação funcional com notação prefixa, sistema de
tipos rico (tipos refinados, enums com payload, dispatch por dominância) e
concorrência via CSP (channels, fibers, select).

O compilador produz código nativo via Cranelift — JIT para desenvolvimento e
AOT para deploy. Sem interpretador, sem VM, sem runtime gerenciado.

```bash
$ kata eval '+ 1 2'
3

$ kata run examples/fatorial.kata
120
```

## Instalação

```bash
git clone https://github.com/arthurjulia/kata.git
cd kata
cargo build --release
# binário em target/release/kata
```

Requer Rust 1.85+ (edition 2024) e C linker (`cc`).

## Primeiros passos

```bash
# Avaliar uma expressão
kata eval '+ 1 2'

# Executar um arquivo
kata run examples/fizzbuzz.kata

# Compilar para executável nativo
kata build examples/fatorial.kata
./fatorial

# REPL interativo
kata repl

# Testes
kata test examples/assertions.kata
```

## A linguagem

### Notação prefixa

Sem infix, sem precedência de operadores. `+ 1 2` é `1 + 2`.

```
+ 1 2              # 3
* (+ 3 4) 2        # 14
```

### Funções e pattern matching

```
fat :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat (- n 1) (* n acc)

fat 5 1            # 120
```

```
fizzbuzz :: Int => Text
lambda x:
    both: "FizzBuzz"
    fizz: "Fizz"
    buzz: "Buzz"
    otherwise: show x
    with
        fizz := = 0 (mod x 3)
        buzz := = 0 (mod x 5)
        both := and fizz buzz
```

### Actions — barreira pure/impure

Funções são puras. Side-effects vivem em actions, marcadas com `!`.

```
action greet
    echo!("hello")
    echo!("world")

greet!()
```

### Higher-order functions e holes

`_` é um hole — cria uma closure parcial.

```
map (* _ 2) [1 2 3]           # [2 4 6]
filter (lambda x: > x 0) [-1 2 -3 4]   # [2 4]
fold (+ _ _) 0 [1 2 3 4 5]   # 15
```

### Coleções

```
[1 2 3]                         # List
(1, "hello", True)              # Tuple
{"nome": "Ana" "idade": 30}     # Dict
{|1 2 3|}                       # Set
[1..1..10]                      # Range (1 a 10, step 1)
```

### Tipos refinados

```
data (Int, > _ 0) as PositiveInt

let x := 5::PositiveInt         # ok
let y := -5::PositiveInt        # type error
```

### CSP — concorrência

```
action produtor (tx::Sender::Int) => Unit
    sleep!(100)
    tx <! 42

action consumidor (rx::Receiver::Int) => Unit
    select
        rx !> v: echo!(v)
        timeout 500: echo!("timeout")

action main => Unit
    let (tx, rx) := channel!()
    fork!(produtor, (tx))
    fork!(consumidor, (rx))
    sleep!(1000)

main!()
```

## Arquitetura

15 crates, ~100K linhas de Rust:

```
kata-core        Tipos canônicos, TypeEnv, DispatchTable
kata-ast         AST
kata-lexer       Lexer
kata-parser      Parser (arity-aware, dois passes)
kata-diagnostics Diagnósticos com spans (miette)
kata-resolution  Name resolution, módulos, imports
kata-inference   Type checker + síntese + desugar
kata-codegen     Lowering TAST → CLIF → Cranelift
kata-optimizer   TCO, TRMA, stream fusion
kata-monomorph   Monomorphization de genéricos
kata-tree-shaking Remoção de código morto
kata-rt          Runtime nativo (arenas, BigInt, CSP, I/O)
kata-comptime    Avaliação em compile-time (@comptime, @cache)
kata-driver      CLI (lex, parse, eval, run, test, build, repl, lsp)
kata-lsp         Language Server Protocol
```

Pipeline: `source → lexer → parser → resolution → inference → optimizer → monomorph → codegen → Cranelift JIT/AOT → runtime`

## Comandos

| Comando | Descrição |
|---------|-----------|
| `kata eval` | Avalia expressão via JIT |
| `kata run` | Compila e executa arquivo via JIT |
| `kata build` | Compila para executável nativo (AOT) |
| `kata test` | Descobre e executa testes `@test` |
| `kata repl` | REPL interativo |
| `kata lex` | Análise léxica (debug) |
| `kata parse` | Análise sintática (debug) |
| `kata lsp` | Servidor LSP para editores |

## Exemplos

```
examples/
  fatorial.kata          Recursão com TCO
  fizzbuzz.kata          Pattern matching + guards
  quicksort.kata         HOFs + List
  map_filter_fold.kata   Higher-order functions
  select_queue.kata      CSP: channels + select + timeout
  refined_types.kata     Tipos refinados + smart constructors
  broadcast.kata         CSP: broadcast channels
  stream_fusion.kata     Stream fusion (map/filter encadeados)
```

## Licença

CC BY-NC 4.0 (Attribution-NonCommercial)