# Capítulo 2 — Adivinhe o Número

Vamos construir algo real: um jogo de adivinhação. O programa escolhe um número aleatório entre 1 e 100, e você tenta adivinhar. A cada tentativa, ele diz se o palpite foi alto demais, baixo demais, ou certo.

Este capítulo introduz vários conceitos de uma vez — actions, pattern matching, leitura de stdin, loops. Não se preocupe em entender cada detalhe agora. Os próximos capítulos deconstruct cada um. O objetivo aqui é sentir a linguagem funcionando.

## Primeiro passo: um número aleatório

Kata tem `rand_int!()` — uma action que gera um inteiro aleatório num intervalo:

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

Cada execução dá um número diferente. O `!` em `rand_int!` indica que é uma *action* — uma função impura que interage com o mundo (neste caso, o gerador de números aleatórios). O capítulo 7 explica actions em detalhe.

## Segundo passo: ler entrada do usuário

Para ler o que o jogador digita, abrimos `/dev/stdin` como arquivo e lemos linha a linha:

```kata
action main
    let r := open!("/dev/stdin", Read)
    match r
        Result::Ok f:
            let r2 := readline!(f)
            match r2
                Result::Ok linha: echo!(linha)
                Result::Err e: echo!("erro de leitura")
        Result::Err e: echo!("erro ao abrir stdin")
main!()
```

```bash
echo "42" | kata run jogo.kata
```

```
42
```

Muita coisa nova aqui. Vamos por partes:

- `open!("/dev/stdin", Read)` abre o stdin como arquivo. Retorna `Result` — um tipo que pode ser `Ok` (sucesso) ou `Err` (erro). O capítulo 6 cobre `match` em detalhe.
- `match r` examina o `Result`. Se for `Ok f`, o arquivo está em `f`. Se for `Err e`, o erro está em `e`.
- `readline!(f)` lê uma linha do arquivo. Também retorna `Result`.
- `echo!(linha)` imprime a linha.

Por que tanto `Result`? Porque operações de I/O podem falhar — o arquivo pode não existir, a leitura pode dar erro. Kata não esconde isso. Você lida com o erro explicitamente via `match`.

## Convertendo texto em número

O que lemos do stdin é `Text`. Para comparar com o número aleatório, precisamos converter para `Int`:

```kata
action main
    let r := open!("/dev/stdin", Read)
    match r
        Result::Ok f:
            let r2 := readline!(f)
            match r2
                Result::Ok linha:
                    let palpite := int linha
                    echo!(palpite)
                Result::Err e: echo!("erro de leitura")
        Result::Err e: echo!("erro ao abrir stdin")
main!()
```

```bash
echo "42" | kata run jogo.kata
```

```
42
```

`int linha` converte `Text` para `Int`. Se a string for `"42"`, vira `42`. O capítulo 3 mostra conversões de tipo em detalhe.

## Comparando o palpite

Kata não tem `if`. Condicionais usam `match` em `Boolean`:

```kata
action main
    let alvo := 42
    let r := open!("/dev/stdin", Read)
    match r
        Result::Ok f:
            let r2 := readline!(f)
            match r2
                Result::Ok linha:
                    let palpite := int linha
                    match (> palpite alvo)
                        Boolean::True: echo!("muito alto")
                        Boolean::False:
                            match (< palpite alvo)
                                Boolean::True: echo!("muito baixo")
                                Boolean::False: echo!("acertou!")
                Result::Err e: echo!("erro de leitura")
        Result::Err e: echo!("erro ao abrir stdin")
main!()
```

```bash
echo "50" | kata run jogo.kata
```

```
muito alto
```

`> palpite alvo` retorna `True` ou `False`. O `match` verifica qual e executa o braço correspondente. Se não for alto demais, verificamos se é baixo demais. Se não for nenhum dos dois, é porque acertou.

## Loop: múltiplas tentativas

Uma tentativa só não é um jogo. Precisamos de um loop que pede palpites até acertar. Kata tem `loop` e `break` (sai do loop):

```kata
action jogar (alvo::Int) => Unit
    let r := open!("/dev/stdin", Read)
    match r
        Result::Ok f:
            loop
                let r2 := readline!(f)
                match r2
                    Result::Ok linha:
                        let palpite := int linha
                        match (> palpite alvo)
                            Boolean::True: echo!("muito alto")
                            Boolean::False:
                                match (< palpite alvo)
                                    Boolean::True: echo!("muito baixo")
                                    Boolean::False:
                                        echo!("acertou!")
                                        break
                    Result::Err e: break
        Result::Err e: echo!("erro ao abrir stdin")

jogar!(42)
```

```bash
printf "50\n30\n42\n" | kata run jogo.kata
```

```
muito alto
muito baixo
acertou!
```

Muita coisa aconteceu aqui:

- `action jogar (alvo::Int) => Unit` define uma action chamada `jogar` que recebe um `Int` e retorna `Unit` (nada).
- `loop` é um laço infinito. `break` sai dele.
- A cada iteração, lemos uma linha do stdin, convertemos para `Int`, e comparamos com o alvo.
- Quando o palpite é correto, `break` sai do loop imediatamente.
- Se `readline!` retorna `Err` (fim do input), `break` também sai do loop.

## Jogo completo: número aleatório

Agora juntamos tudo — o número alvo é gerado aleatoriamente:

```kata
action jogar (alvo::Int) => Unit
    let r := open!("/dev/stdin", Read)
    match r
        Result::Ok f:
            loop
                let r2 := readline!(f)
                match r2
                    Result::Ok linha:
                        let palpite := int linha
                        match (> palpite alvo)
                            Boolean::True: echo!("muito alto")
                            Boolean::False:
                                match (< palpite alvo)
                                    Boolean::True: echo!("muito baixo")
                                    Boolean::False:
                                        echo!("acertou!")
                                        break
                    Result::Err e: break
        Result::Err e: echo!("erro ao abrir stdin")

jogar!(rand_int!(1, 100))
```

```bash
printf "50\n25\n37\n44\n40\n42\n" | kata run jogo.kata
```

```
muito baixo
muito alto
muito alto
muito baixo
muito baixo
acertou!
```

O output acima é de uma partida onde o alvo era 42. Como o número é aleatório, a sequência de dicas muda a cada execução — o que torna o jogo rejugável. Tente algumas vezes.

## O que você aprendeu

Você construiu um jogo interativo completo. No caminho, tocou em:

- **Actions** — funções impuras com `!` (`rand_int!`, `open!`, `readline!`, `echo!`)
- **`Result`** — success ou erro, sempre via `match`
- **`match`** — condicional sem `if`, examinando a forma do valor
- **`loop` e `break`** — iteração e saída de loop
- **`int`** — conversão de `Text` para `Int`

Cada um desses conceitos é coberto em profundidade nos próximos capítulos. O capítulo 3 mostra a sintaxe básica. O capítulo 7 explica actions, `var`, e `loop`. O capítulo 6 cobre `match` em detalhe.

Por enquanto, você já escreveu um programa real em Kata. Isso é mais do que a maioria das linguagens oferece no primeiro dia.