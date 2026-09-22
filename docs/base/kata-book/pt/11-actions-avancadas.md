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

`channel!` cria um canal síncrono (rendezvous). O operador de transmissão `<!` bloqueia até o `!>` sincronizar. Retorna um par `(Sender, Receiver)`:

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

## `select` com sockets

`select` não é limitado a canais — também multiplexa sockets e file
handles. Um servidor pode ler de múltiplas fontes concorrentemente:

```kata
action extrair_n (r::ReadResult::(Bytes)) => Int
    match r
        Data bytes: len bytes
        Error _: -1
        Eof: -2

action fazer_select (conn::Socket, tx::Sender::Int) => Unit
    select
        read!(conn, 100) !> dados: tx <! extrair_n!(dados)

action servidor (listener::Socket, tx::Sender::Int) => Unit
    match (accept!(listener))
        Ok conn: fazer_select!(conn, tx)
        Err _: tx <! -2

action main => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Listener))
        Ok listener:
            let (tx, rx) := channel!()
            fork!(servidor, (listener, tx))
            rx !> n
            echo!(n)
        Err msg: echo!(msg)
main!()
```

`accept!(listener)` retorna `Result::(Socket, Text)` e fica fora do
`select`. Dentro do `select`, `read!(conn, 100) !> dados` aguarda dados
do socket — o primeiro braço pronto vence.

## `queue!` — canal bufferizado

`channel!` é síncrono (rendezvous): o operador de transmissão `<!` bloqueia até o `!>` sincronizar. `queue!(N)` cria um canal com buffer de capacidade N — o `<!` não bloqueia enquanto houver espaço no buffer:

```kata
action produtor (tx::Sender::Int) => Unit
    tx <! 10
    tx <! 20
    tx <! 30

action main => Int
    let (tx, rx) := queue!(3)
    fork!(produtor, (tx,))
    rx !> a
    rx !> b
    rx !> c
    + a + b c
main!()
```

```
60
```

O produtor envia três valores sem bloquear — o buffer comporta todos. O consumidor recebe sequencialmente. Se o buffer encher, o próximo `<!` bloqueia até o consumidor drenar.

## `broadcast!` — um-para-muitos

`broadcast!()` cria um canal fire-and-forget. O retorno é `(Sender, ReceiverFactory)` — não `(Sender, Receiver)`. `ReceiverFactory` é uma fábrica: cada chamada `rxf!()` produz um novo `Receiver` independente. Todos os receivers veem o último valor enviado (semântica *latest only*):

```kata
action main => Int
    let (tx, rxf) := broadcast!()
    let rx1 := rxf!()
    let rx2 := rxf!()
    tx <! 42
    rx1 !> a
    rx2 !> b
    b
main!()
```

```
42
```

Como `rx1` e `rx2` são receivers independentes da mesma fonte, cada um recebe o valor `42`. O `Sender` não espera receivers — envio é não-bloqueante. Receivers criados depois de um envio não vêem mensagens passadas (future-only).

A semântica é *latest only*: se múltiplos valores são enviados antes de um receiver ler, ele vê apenas o último. Não há fila — é um latch, não um buffer.

## Topologia de canais

Canais conectam fibers na árvore de `fork!`. A comunicação acontece em duas direções:

- **Pai → filho:** o pai cria o canal, passa o `Sender` ou `Receiver` como argumento de `fork!`.
- **Irmão → irmão:** o pai cria o canal, passa o `Sender` para uma filha e o `Receiver` para outra.

`Sender`, `Receiver` e `ReceiverFactory` fluem apenas descendente — do pai para os filhos via argumentos de `fork!`. Eles são handles de operadores, não valores de dados: não trafegam por `<!` nem são retornados de actions. Essa restrição garante que o ancestral comum de dois fibers conectados por canal é sempre o pai direto do sender — o que permite ao compilador alocar os valores na arena correta em compile-time, sem cópia nem garbage collector.

## `spawn!` — processo OS isolado

`fork!` cria um fiber — uma corrotina leve no mesmo processo. `spawn!` vai além: cria um **processo OS separado** via `fork` do sistema operacional. O child herda a arena via copy-on-write e executa a action isoladamente.

A diferença fundamental: `fork!` compartilha memória com o parent; `spawn!` não. O child é um processo distinto — falhas (crash,segfault) no child não afetam o parent.

`spawn!` é fire-and-forget — não há retorno de valor. A comunicação entre parent e child é por canais IPC (pipe Unix):

```kata
action worker (rx::Receiver::Int, tx2::Sender::Int) => Int
    rx !> n
    tx2 <! + n 1
    0

action main => Int
    let ch1 := channel!()
    let tx1 := ch1.0
    let rx1 := ch1.1
    let ch2 := channel!()
    let tx2 := ch2.0
    let rx2 := ch2.1
    spawn!(worker, (rx1, tx2))
    tx1 <! 42
    rx2 !> result
    result
main!()
```

```
43
```

O parent cria dois canais: `ch1` (parent→child) e `ch2` (child→parent). `spawn!(worker, (rx1, tx2))` inicia o worker em processo separado, passando o receiver de `ch1` e o sender de `ch2`. O parent envia `42` via `tx1`, o child recebe via `rx1`, incrementa, envia `43` via `tx2`, e o parent recebe via `rx2`.

A sintaxe é idêntica à de `fork!`: `spawn!(action, (args))`. A diferença é semântica — processo OS vs fiber.

### `fork!` vs `spawn!`

| | `fork!` | `spawn!` |
|---|---|---|
| Unidade | Fiber (corrotina) | Processo OS |
| Memória | Compartilhada (mesma arena) | Isolada (COW) |
| Comunicação | Canais na mesma memória | Canais IPC (pipe Unix) |
| Falha | Crash no fiber derruba o processo | Crash no child não afeta parent |
| Retorno | Cooperativo via canal | Fire-and-forget |
| Plataforma | Linux, macOS, Windows | Linux, macOS (stub no Windows) |

Use `fork!` para concorrência leve dentro do mesmo processo. Use `spawn!` para isolamento — quando um worker pode crashar ou precisar de memória própria.

## `sleep!` — yield cooperativo

`sleep!(ms)` suspende o fiber atual por N milissegundos, cedendo o controle ao scheduler. É a forma de esperar sem bloquear a thread.

## Limitações no Windows

`fork!`, canais, `select`, e `sleep!` funcionam em todas as plataformas. No entanto, `spawn!` — que cria processos filhos isolados do sistema operacional — é um stub no Windows: compila, mas em runtime não faz nada (retorna 0). Se você precisa de processos externos, use Linux ou macOS. Veja o [Apêndice — Plataformas e Limitações](17-plataformas-limitacoes.md) para detalhes.

## Sockets — I/O de rede

Sockets em Kata são handles opacos (`Socket`) para conexões TCP ou Unix.
A API segue o modelo BSD/POSIX: `open!` cria, `accept!` aceita, `read!`/`write!`
transportam, `close!` fecha.

### Criar um listener

```kata
action main => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Listener))
        Ok listener:
            echo!("servidor ouvindo")
            let _ := close!(listener)
        Err msg: echo!(msg)
main!()
```

`SocketKind::TCP(addr)` ou `SocketKind::Unix(path)`.
`SocketMode::Listener` (passivo — espera conexões) ou `SocketMode::Connected` (ativo — conecta a um servidor).

### Aceitar conexões

`accept!(listener)` bloqueia até um cliente conectar. Retorna
`Result::(Socket, Text)` — o socket `Connected` do cliente:

```kata
action main => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Listener))
        Ok listener:
            match (accept!(listener))
                Ok conn: echo!("cliente conectado")
                Err msg: echo!(msg)
            let _ := close!(listener)
        Err msg: echo!(msg)
main!()
```

### Ler e escrever

`read!(conn)` retorna `ReadResult::(Bytes)` — tri-valorado:
`Data(bytes)` (dado lido), `Error(msg)` (falha), `Eof` (cliente
desconectou). `readline!(conn)` retorna `ReadResult::(Text)`:

```kata
action eco (conn::Socket) => Unit
    loop
        match (readline!(conn))
            ReadResult::Data linha:
                let _ := write!(conn, linha)
            ReadResult::Error _:
                echo!("erro")
                break
            ReadResult::Eof:
                echo!("fim")
                break
```

`write!(conn, content)` envia `Text` ou `Bytes` — retorna
`Result::(Unit, Text)`.

### Conectar a um servidor

```kata
action cliente => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Connected))
        Ok conn:
            let _ := write!(conn, "olá")
            echo!("enviado")
            let _ := close!(conn)
        Err msg: echo!(msg)
```

### Fechar

`close!(socket)` fecha o handle. Não retorna `Result` — sempre
sucede.

## Fim

Você completou a parte principal do Kata Book. Dos literais à concorrência — sem `if`, sem classes, sem herança. Kata é pequena por design: notação prefixa, pattern matching, e tipos algébricos resolvem o que outras linguagens espalham por dezenas de features.

→ [Capítulo 12 — Tipos Refinados e Alias](12-tipos-refinados.md)