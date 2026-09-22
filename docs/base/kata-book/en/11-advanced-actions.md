# Chapter 11 — Advanced Actions

Kata has cooperative concurrency based on CSP (Communicating Sequential Processes). Fibers — lightweight coroutines — communicate via channels. The scheduler is single-threaded: cooperative yielding, not preemption.

## `fork!` — creating fibers

`fork!` submits an action to run as an isolated fiber:

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

`fork!` takes the action and a tuple with the arguments. The fiber runs concurrently — `sleep!(50)` in the main action gives time for the worker to execute.

## Channels — `channel!`

`channel!` creates a synchronous (rendezvous) channel. The send operator `<!` blocks until the `!>` synchronizes. It returns a pair `(Sender, Receiver)`:

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

The producer sends `()` via `tx <! ()`. The consumer receives via `rx !> valor`. The `!>` blocks the fiber until a value arrives.

## `select` — multiplexing

`select` waits on multiple channels and executes the first one to receive. `timeout` is a special arm that fires after N milliseconds:

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

Nobody sent on the channel, so `timeout 100` fires first. With a producer:

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

The producer sends after 50ms — `select` receives before the 100ms timeout.

## `select` with sockets

`select` is not limited to channels — it also multiplexes sockets and file
handles. A server can read from multiple sources concurrently:

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

`accept!(listener)` returns `Result::(Socket, Text)` and stays outside the
`select`. Inside the `select`, `read!(conn, 100) !> dados` awaits data
from the socket — the first ready arm wins.

## `queue!` — buffered channel

`channel!` is synchronous (rendezvous): the send operator `<!` blocks until the `!>` synchronizes. `queue!(N)` creates a channel with a buffer of capacity N — `<!` does not block while there is space in the buffer:

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

The producer sends three values without blocking — the buffer holds them all. The consumer receives sequentially. If the buffer fills, the next `<!` blocks until the consumer drains it.

## `broadcast!` — one-to-many

`broadcast!()` creates a fire-and-forget channel. The return is `(Sender, ReceiverFactory)` — not `(Sender, Receiver)`. `ReceiverFactory` is a factory: each call `rxf!()` produces a new independent `Receiver`. All receivers see the last sent value (*latest only* semantics):

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

Since `rx1` and `rx2` are independent receivers from the same source, each receives the value `42`. The `Sender` does not wait for receivers — sending is non-blocking. Receivers created after a send do not see past messages (future-only).

The semantics are *latest only*: if multiple values are sent before a receiver reads, it sees only the last one. There is no queue — it is a latch, not a buffer.

## Channel topology

Channels connect fibers in the `fork!` tree. Communication happens in two directions:

- **Parent → child:** the parent creates the channel, passes the `Sender` or `Receiver` as a `fork!` argument.
- **Sibling → sibling:** the parent creates the channel, passes the `Sender` to one child and the `Receiver` to another.

`Sender`, `Receiver`, and `ReceiverFactory` flow only downward — from parent to children via `fork!` arguments. They are operator handles, not data values: they do not travel through `<!` nor are they returned from actions. This restriction ensures that the common ancestor of two fibers connected by a channel is always the direct parent of the sender — which allows the compiler to allocate values in the correct arena at compile-time, with no copying and no garbage collector.

## `spawn!` — isolated OS process

`fork!` creates a fiber — a lightweight coroutine in the same process. `spawn!` goes further: it creates a **separate OS process** via the operating system's `fork`. The child inherits the arena via copy-on-write and runs the action in isolation.

The fundamental difference: `fork!` shares memory with the parent; `spawn!` does not. The child is a distinct process — failures (crash, segfault) in the child do not affect the parent.

`spawn!` is fire-and-forget — there is no value return. Communication between parent and child is via IPC channels (Unix pipe):

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

The parent creates two channels: `ch1` (parent→child) and `ch2` (child→parent). `spawn!(worker, (rx1, tx2))` starts the worker in a separate process, passing the receiver from `ch1` and the sender from `ch2`. The parent sends `42` via `tx1`, the child receives via `rx1`, increments, sends `43` via `tx2`, and the parent receives via `rx2`.

The syntax is identical to `fork!`: `spawn!(action, (args))`. The difference is semantic — OS process vs fiber.

### `fork!` vs `spawn!`

| | `fork!` | `spawn!` |
|---|---|---|
| Unit | Fiber (coroutine) | OS process |
| Memory | Shared (same arena) | Isolated (COW) |
| Communication | Channels in same memory | IPC channels (Unix pipe) |
| Failure | Fiber crash brings down the process | Child crash does not affect parent |
| Return | Cooperative via channel | Fire-and-forget |
| Platform | Linux, macOS, Windows | Linux, macOS (stub on Windows) |

Use `fork!` for lightweight concurrency within the same process. Use `spawn!` for isolation — when a worker may crash or needs its own memory.

## `sleep!` — cooperative yield

`sleep!(ms)` suspends the current fiber for N milliseconds, yielding control to the scheduler. It is the way to wait without blocking the thread.

## Limitations on Windows

`fork!`, channels, `select`, and `sleep!` work on all platforms. However, `spawn!` — which creates isolated OS child processes — is a stub on Windows: it compiles, but at runtime does nothing (returns 0). If you need external processes, use Linux or macOS. See the [Appendix — Platforms and Limitations](17-platforms-limitations.md) for details.

## Sockets — network I/O

Sockets in Kata are opaque handles (`Socket`) for TCP or Unix connections.
The API follows the BSD/POSIX model: `open!` creates, `accept!` accepts, `read!`/`write!`
transfer, `close!` closes.

### Creating a listener

```kata
action main => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Listener))
        Ok listener:
            echo!("servidor ouvindo")
            let _ := close!(listener)
        Err msg: echo!(msg)
main!()
```

`SocketKind::TCP(addr)` or `SocketKind::Unix(path)`.
`SocketMode::Listener` (passive — waits for connections) or `SocketMode::Connected` (active — connects to a server).

### Accepting connections

`accept!(listener)` blocks until a client connects. Returns
`Result::(Socket, Text)` — the client's `Connected` socket:

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

### Reading and writing

`read!(conn)` returns `ReadResult::(Bytes)` — tri-valued:
`Data(bytes)` (data read), `Error(msg)` (failure), `Eof` (client
disconnected). `readline!(conn)` returns `ReadResult::(Text)`:

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

`write!(conn, content)` sends `Text` or `Bytes` — returns
`Result::(Unit, Text)`.

### Connecting to a server

```kata
action cliente => Unit
    match (open!(SocketKind::TCP("127.0.0.1:8080"), SocketMode::Connected))
        Ok conn:
            let _ := write!(conn, "olá")
            echo!("enviado")
            let _ := close!(conn)
        Err msg: echo!(msg)
```

### Closing

`close!(socket)` closes the handle. It does not return `Result` — it always
succeeds.

## End

You have completed the main part of the Kata Book. From literals to concurrency — no `if`, no classes, no inheritance. Kata is small by design: prefix notation, pattern matching, and algebraic types solve what other languages spread across dozens of features.

→ [Chapter 12 — Refined Types and Aliases](12-refined-types.md)