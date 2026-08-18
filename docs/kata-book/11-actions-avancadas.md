# Capítulo 11 — Actions Avançadas

Kata tem concorrência cooperativa baseada em CSP (Communicating Sequential Processes). Fibers — corrotinas leves — comunicam via canais. O scheduler é single-threaded: yield cooperativo, não preempção.

## `fork!` — criando fibers

`fork!` submete uma action para executar como fiber isolado:

```kata
action worker (n::Int) => Unit
    echo!(n)

action main => Unit
    fork!(worker, (42,))
    sleep!(50)
main!()
```

```
42
```

O `fork!` recebe a action e uma tupla com os argumentos. O fiber roda concorrentemente — `sleep!(50)` na action principal dá tempo para o worker executar.

## Canais — `channel!`

`channel!` cria um canal síncrono (rendezvous). O envio `<!` bloqueia até o receptor `!>` sincronizar. Retorna um par `(Sender, Receiver)`:

```kata
action produtor (tx::Sender::Unit) => Unit
    tx <! ()
    echo!("enviado")

action consumidor (rx::Receiver::Unit) => Unit
    rx !> valor
    echo!("recebido")

action main => Unit
    let (tx, rx) := channel!()
    fork!(produtor, (tx,))
    fork!(consumidor, (rx,))
    sleep!(100)
main!()
```

```
enviado
recebido
```

O produtor envia `()` via `tx <! ()`. O consumidor recebe via `rx !> valor`. O `!>` bloqueia o fiber até chegar um valor.

## `select` — multiplexação

`select` espera em múltiplos canais e executa o primeiro que receber. `timeout` é um braço especial que dispara após N milissegundos:

```kata
action worker (rx::Receiver::Unit) => Unit
    select
        rx !> valor: echo!("recebeu")
        timeout 100: echo!("timeout")

action main => Unit
    let (tx, rx) := channel!()
    fork!(worker, (rx,))
    sleep!(200)
main!()
```

```
timeout
```

Ninguém enviou no canal, então o `timeout 100` disprime primeiro. Com um produtor:

```kata
action produtor (tx::Sender::Unit) => Unit
    sleep!(50)
    tx <! ()

action consumidor (rx::Receiver::Unit) => Unit
    select
        rx !> valor: echo!("recebeu")
        timeout 100: echo!("timeout")

action main => Unit
    let (tx, rx) := channel!()
    fork!(produtor, (tx,))
    fork!(consumidor, (rx,))
    sleep!(200)
main!()
```

```
recebeu
```

O produtor envia após 50ms — o `select` recebe antes do timeout de 100ms.

## `sleep!` — yield cooperativo

`sleep!(ms)` suspende o fiber atual por N milissegundos, cedendo o controle ao scheduler. É a forma de esperar sem bloquear a thread.

## Fim

Você completou a parte principal do Kata Book. Dos literais à concorrência — sem `if`, sem classes, sem herança. Kata é pequena por design: notação prefixa, pattern matching, e tipos algébricos resolvem o que outras linguagens espalham por dezenas de features.

Para aprofundar, explore o `examples/` no repositório e o manual técnico em `docs/Kata-lang-manual.md`.