# Chapter 2 — Guess the Number

Let's build something real: a guessing game. The program picks a random number between 1 and 100, and you try to guess it. Each attempt, it tells you whether your guess was too high, too low, or correct.

This chapter introduces several concepts at once — actions, pattern matching, reading stdin, loops, pure functions. Don't worry about understanding every detail now. The next chapters deconstruct each one. The goal here is to feel the language working.

## First step: a random number

Kata has `rand_int!()` — an action that generates a random integer in a range:

```kata
action main
    echo!(rand_int!(1, 100))
main!()
```

```bash
kata run jogo.kata
```

```
42
```

Each run gives a different number. The `!` in `rand_int!` indicates it's an *action* — an impure function that interacts with the world (in this case, the random number generator). Chapter 7 explains actions in detail.

## Second step: reading user input

To read what the player types, we use `input!()` — an action that shows a prompt and reads a line from stdin:

```kata
action main
    let linha := input!("Palpite: ")
    echo!(linha)
main!()
```

```bash
echo "42" | kata run jogo.kata
```

```
Palpite: 42
```

A lot of new things here. Let's go step by step:

- `input!("Palpite: ")` prints `"Palpite: "` to the terminal and reads a line from stdin. It returns `Text` — what was typed, without the trailing `\n`.
- The `!` in `input!` indicates it's an *action* — an impure function that interacts with the world (in this case, stdin and stdout).
- `let linha := ...` creates an immutable binding. Chapter 3 covers `let` in detail.
- `echo!(linha)` prints the line.

Why does `input!` return `Text` directly, without `Result`? Because `input!` is sugar for the common case: if stdin ends (EOF) or there's a read error, it returns empty `Text` (`""`). If you need to handle errors explicitly, you can use `readline!` with a file handle — chapter 12 shows that.

## Converting text to a number

What we read from stdin is `Text`. To compare with the random number, we need to convert it to `Int`. But the user can type anything — not just numbers. `int` returns `Result`:

```kata
action main
    let linha := input!("Palpite: ")
    let r := int(linha)
    match r
        Ok n: echo!(n)
        Err e: echo!("não é um número")
main!()
```

```bash
echo "42" | kata run jogo.kata
```

```
Palpite: 42
```

```bash
echo "abc" | kata run jogo.kata
```

```
Palpite: não é um número
```

- `int(linha)` tries to convert `Text` to `Int`. It returns `Result::(Int, Text)` — `Ok(n)` if the string is a valid number, `Err("número inválido")` if it's not.
- `match r` examines the `Result`. If it's `Ok n`, the number is in `n`. If it's `Err e`, the error is in `e`.
- `int` is a pure function — it has no side effects. Failure is represented as a value (`Result`), not as an effect. The user can type anything; `int` returns `Ok(n)` or `Err("número inválido")`, and you decide what to do with `match` or `|`.

So much `Result`? Because operations that can fail shouldn't crash the program. The user typed "abc"? That's fine — you handle the error explicitly via `match`. Chapter 6 covers `match` in detail.

## The `|` operator (fallback)

The `match` above is verbose when you just want a default value. Kata has the `|` operator — it unwraps the `Ok` and uses the right-hand side as a fallback if it's `Err`:

```kata
action main
    let linha := input!("Palpite: ")
    let n := int(linha) | 0
    echo!(n)
main!()
```

```bash
echo "42" | kata run jogo.kata
```

```
Palpite: 42
```

```bash
echo "abc" | kata run jogo.kata
```

```
Palpite: 0
```

`int(linha) | 0` means: "try to convert `linha` to Int; if it fails, use `0`". Much more direct than a full `match` when you just need a fallback.

## First version of the game: simple

We already have all the pieces for a working game. Kata has no `if` — conditionals use `match` on `Boolean`:

```kata
action jogar (alvo::Int) => Unit
    loop
        let palpite := int(input!("Palpite: ")) | 0
        match (> palpite alvo)
            True: echo!("muito alto")
            False:
                match (< palpite alvo)
                    True: echo!("muito baixo")
                    False:
                        echo!("acertou!")
                        break

jogar!(rand_int!(1, 100))
```

```bash
printf "50\n25\n37\n42\n" | kata run jogo.kata
```

```
Palpite: muito alto
Palpite: muito baixo
Palpite: muito baixo
Palpite: acertou!
```

Let's break it down:

- `action jogar (alvo::Int) => Unit` defines an action called `jogar` that takes an `Int` and returns `Unit` (nothing).
- `loop` is an infinite loop. `break` exits it.
- `int(input!("Palpite: ")) | 0` composes three operations: reads input, converts to Int, and if it fails uses `0`. All in one line — no nested `match` for the `Result`.
- `> palpite alvo` returns `True` or `False`. The `match` checks which one and executes the corresponding arm. If it's not too high, we check if it's too low. If it's neither, it's because we got it right.

The cost: if the user types "abc", the guess silently becomes `0`. The game says "muito baixo" instead of "não é um número". For a quick game, that's fine. For something robust, you want to handle the error explicitly — we'll come back to that.

## Second version: handling invalid input

The version with `|` is simple, but it swallows errors — "abc" becomes `0` and the game says "muito baixo" without explanation. To handle the error explicitly, we use `match` on the `Result`:

```kata
action jogar (alvo::Int) => Unit
    loop
        let linha := input!("Palpite: ")
        let r := int(linha)
        match r
            Ok palpite:
                match (> palpite alvo)
                    True: echo!("muito alto")
                    False:
                        match (< palpite alvo)
                            True: echo!("muito baixo")
                            False:
                                echo!("acertou!")
                                break
            Err e: echo!("não é um número")

jogar!(rand_int!(1, 100))
```

```bash
printf "abc\n50\n30\n42\n" | kata run jogo.kata
```

```
Palpite: não é um número
Palpite: muito alto
Palpite: muito baixo
Palpite: acertou!
```

The difference: `match r` examines the `Result`. If it's `Ok palpite`, the number is in `palpite` and the game continues. If it's `Err e`, we show "não é um número" and the loop asks for another attempt.

It works, but the nesting is deep — `Result` → `Boolean` (high) → `Boolean` (low) — three levels of indented `match`. Each level has a purpose, but reading them together takes effort. Worse: the comparison logic is mixed with the I/O and flow control logic. If tomorrow we want to reuse the comparison (e.g., in a different game mode), we'd have to duplicate it.

## Third version: decomposing with functions

The problem with the previous version isn't lack of features — it's lack of decomposition. The logic for "comparing two numbers and saying whether the guess is high, low, or correct" has nothing to do with I/O. It's a pure function. Let's extract it:

```kata
comparar :: Int Int => Optional::Text
lambda palpite alvo:
    > palpite alvo: Some "muito alto"
    < palpite alvo: Some "muito baixo"
    otherwise: None
```

`comparar` is a pure function — no `!`, no `action`. It takes two `Int`s and returns `Optional::Text`:

- `Some "muito alto"` if the guess is greater than the target
- `Some "muito baixo"` if it's less
- `None` if correct (no hint to give)

`Optional::Text` means "maybe a `Text`". `Some` carries the value; `None` means absence. The function is pure because it doesn't depend on the state of the world — same input, same output, always.

Now the part involving I/O. Reading input and converting to Int can fail — the user might type "abc". But instead of propagating the error, we can handle it within the action itself: if the input is invalid, we warn and ask again. The `loop` does this naturally:

```kata
action ler_palpite => Int
    loop
        match int(input!("Palpite: "))
            Ok n: return n
            Err e:
                echo!("não é um número")
                continue
```

`ler_palpite` only returns when the input is valid. The `loop` asks for guesses until the user types a number. `return n` exits the action (not just the loop) with the number. `continue` goes back to the start of the loop — asks for another attempt.

This means `ler_palpite` always returns a valid `Int`. It never fails. The input error is handled internally, not propagated.

With `comparar` and `ler_palpite` extracted, `jogar` becomes shallow:

```kata
action jogar (alvo::Int) => Unit
    loop
        let palpite := ler_palpite!()
        match comparar palpite alvo
            Some msg: echo!(msg)
            None:
                echo!("acertou!")
                break

jogar!(rand_int!(1, 100))
```

```bash
printf "abc\n50\n30\n42\n" | kata run jogo.kata
```

```
Palpite: não é um número
Palpite: muito alto
Palpite: muito baixo
Palpite: acertou!
```

One level of `match` — just the `comparar`. No `match` on `Result`, no `match` on `Boolean`, no nesting.

The complete code:

```kata
action ler_palpite => Int
    loop
        match int(input!("Palpite: "))
            Ok n: return n
            Err e:
                echo!("não é um número")
                continue

comparar :: Int Int => Optional::Text
lambda palpite alvo:
    > palpite alvo: Some "muito alto"
    < palpite alvo: Some "muito baixo"
    otherwise: None

action jogar (alvo::Int) => Unit
    loop
        let palpite := ler_palpite!()
        match comparar palpite alvo
            Some msg: echo!(msg)
            None:
                echo!("acertou!")
                break

jogar!(rand_int!(1, 100))
```

```bash
printf "50\n25\n37\n44\n40\n42\n" | kata run jogo.kata
```

```
Palpite: muito alto
Palpite: muito baixo
Palpite: muito baixo
Palpite: muito alto
Palpite: muito alto
Palpite: acertou!
```

The output above is from a game where the target was 42. Since the number is random, the sequence of hints changes each run — which makes the game replayable. Try it a few times.

### Why decompose?

The decomposed version is larger — more lines, more definitions. But each piece does one thing:

- `comparar` is pure — you can test it in isolation, without mocking stdin. You can reuse it in a different game mode (e.g., two players).
- `ler_palpite` encapsulates I/O + conversion + validation — the inner loop handles the error and only returns a valid `Int`.
- `jogar` orchestrates — loop, match, and flow control. A single `match`, one level.

The nested version (second) isn't "wrong" — it's the natural starting point when you're learning. Decomposition is the next step: when nesting starts getting in the way of reading, it's time to extract.

## What you learned

You built a complete interactive game. Along the way, you touched:

- **Actions** — impure functions with `!` (`rand_int!`, `input!`, `echo!`)
- **Pure functions** — no `!`, no side effects (`comparar`, `int`, `float`)
- **`Result`** — success or error, always via `match`
- **`Optional`** — presence or absence of a value (`Some` / `None`)
- **`|`** — fallback operator: unwraps `Ok`, uses the right-hand side if `Err`
- **`match`** — conditionals without `if`, examining the shape of the value
- **`loop`, `break`, `continue` and `return`** — iteration, loop exit, and action exit

Each of these concepts is covered in depth in the next chapters. Chapter 3 shows the basic syntax. Chapter 7 explains actions, `var`, and `loop`. Chapter 6 covers `match` in detail. File I/O (`open!`, `readline!`) and sockets are documented in the technical manual (`docs/Kata-lang-manual.md`).

For now, you've already written a real program in Kata — in three versions, from simplest to most decomposed. That's more than most languages offer on the first day.

→ [Chapter 3 — Basic Syntax](03-sintaxe-basica.md)