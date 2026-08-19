# Capítulo 2 — Adivinhe o Número

Vamos construir algo real: um jogo de adivinhação. O programa escolhe um número aleatório entre 1 e 100, e você tenta adivinhar. A cada tentativa, ele diz se o palpite foi alto demais, baixo demais, ou certo.

Este capítulo introduz vários conceitos de uma vez — actions, pattern matching, leitura de stdin, loops, funções puras. Não se preocupe em entender cada detalhe agora. Os próximos capítulos deconstruct cada um. O objetivo aqui é sentir a linguagem funcionando.

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

Para ler o que o jogador digita, usamos `input!()` — uma action que mostra um prompt e lê uma linha do stdin:

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

Muita coisa nova aqui. Vamos por partes:

- `input!("Palpite: ")` imprime `"Palpite: "` no terminal e lê uma linha do stdin. Retorna `Text` — o que foi digitado, sem o `\n` final.
- O `!` em `input!` indica que é uma *action* — função impura que interage com o mundo (neste caso, o stdin e stdout).
- `let linha := ...` cria um binding imutável. O capítulo 3 cobre `let` em detalhe.
- `echo!(linha)` imprime a linha.

Por que `input!` retorna `Text` direto, sem `Result`? Porque `input!` é açúcar para o caso comum: se o stdin acabar (EOF) ou houver erro de leitura, ela devolve `Text` vazio (`""`). Quem precisa tratar erro explicitamente pode usar `readline!` com um handle de arquivo — o capítulo 12 mostra isso.

## Convertendo texto em número

O que lemos do stdin é `Text`. Para comparar com o número aleatório, precisamos converter para `Int`. Mas o usuário pode digitar qualquer coisa — não apenas números. `int!` retorna `Result`:

```kata
action main
    let linha := input!("Palpite: ")
    let r := int!(linha)
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

- `int!(linha)` tenta converter `Text` para `Int`. Retorna `Result::(Int, Text)` — `Ok(n)` se a string é um número válido, `Err("número inválido")` se não é.
- `match r` examina o `Result`. Se for `Ok n`, o número está em `n`. Se for `Err e`, o erro está em `e`.
- `int` é uma action porque pode falhar — o usuário pode digitar qualquer coisa. O `!` sinaliza isso: "esta operação pode dar errado, trate o erro".

Por que tanto `Result`? Porque operações que podem falhar não deveriam crashar o programa. O usuário digitou "abc"? Tudo bem — você lida com o erro explicitamente via `match`. O capítulo 6 cobre `match` em detalhe.

## O operador `|` (fallback)

O `match` acima é verboso quando você só quer um valor default. Kata tem o operador `|` — desempacota o `Ok` e usa o lado direito como fallback se for `Err`:

```kata
action main
    let linha := input!("Palpite: ")
    let n := int!(linha) | 0
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

`int!(linha) | 0` significa: "tente converter `linha` para Int; se falhar, use `0`". Muito mais direto que um `match` completo quando você só precisa de um fallback.

## Primeira versão do jogo: simples

Já temos todas as peças para um jogo funcional. Kata não tem `if` — condicionais usam `match` em `Boolean`:

```kata
action jogar (alvo::Int) => Unit
    loop
        let palpite := int!(input!("Palpite: ")) | 0
        match (> palpite alvo)
            Boolean::True: echo!("muito alto")
            Boolean::False:
                match (< palpite alvo)
                    Boolean::True: echo!("muito baixo")
                    Boolean::False:
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

Vamos destrinchar:

- `action jogar (alvo::Int) => Unit` define uma action chamada `jogar` que recebe um `Int` e retorna `Unit` (nada).
- `loop` é um laço infinito. `break` sai dele.
- `int!(input!("Palpite: ")) | 0` compõe três operações: lê o input, converte para Int, e se falhar usa `0`. Tudo numa linha — sem `match` aninhado para o `Result`.
- `> palpite alvo` retorna `True` ou `False`. O `match` verifica qual e executa o braço correspondente. Se não for alto demais, verificamos se é baixo demais. Se não for nenhum dos dois, é porque acertou.

O custo: se o usuário digita "abc", o palpite vira `0` silenciosamente. O jogo diz "muito baixo" em vez de "não é um número". Para um jogo rápido, tudo bem. Para algo robusto, você quer tratar o erro explicitamente — voltaremos a isso.

## Segunda versão: tratando input inválido

A versão com `|` é simples, mas engole erros — "abc" vira `0` e o jogo diz "muito baixo" sem explicar. Para tratar o erro explicitamente, usamos `match` no `Result`:

```kata
action jogar (alvo::Int) => Unit
    loop
        let linha := input!("Palpite: ")
        let r := int!(linha)
        match r
            Ok palpite:
                match (> palpite alvo)
                    Boolean::True: echo!("muito alto")
                    Boolean::False:
                        match (< palpite alvo)
                            Boolean::True: echo!("muito baixo")
                            Boolean::False:
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

A diferença: `match r` examina o `Result`. Se for `Ok palpite`, o número está em `palpite` e o jogo continua. Se for `Err e`, mostramos "não é um número" e o loop pede outra tentativa.

Funciona, mas o aninhamento é profundo — `Result` → `Boolean` (alto) → `Boolean` (baixo) — três níveis de `match` indentados. Cada nível tem um propósito, mas lê-los juntos exige esforço. Pior: a lógica de comparação está misturada com a lógica de I/O e controle de fluxo. Se amanhã quisermos reusar a comparação (e.g. num modo de jogo diferente), teríamos que duplicá-la.

## Terceira versão: decompondo com funções

O problema da versão anterior não é falta de features — é falta de decomposição. A lógica de "comparar dois números e dizer se o palpite é alto, baixo, ou certo" não tem nada a ver com I/O. É uma função pura. Vamos extraí-la:

```kata
comparar :: Int Int => Optional::Text
lambda palpite alvo:
    > palpite alvo: Some "muito alto"
    < palpite alvo: Some "muito baixo"
    otherwise: None
```

`comparar` é uma função pura — sem `!`, sem `action`. Recebe dois `Int`s e retorna `Optional::Text`:

- `Some "muito alto"` se o palpite é maior que o alvo
- `Some "muito baixo"` se é menor
- `None` se acertou (não há dica a dar)

`Optional::Text` significa "talvez um `Text`". `Some` carrega o valor; `None` significa ausência. A função é pura porque não depende do estado do mundo — mesma entrada, mesma saída, sempre.

Da mesma forma, a lógica de "ler uma linha do stdin e converter para Int" é uma action que pode falhar. Vamos extraí-la também:

```kata
action ler_palpite => Result::(Int, Text)
    let n := int!(input!("Palpite: ")) ?
    Ok n
```

O `?` é o operador de fail-fast: desempacota o `Result` — se for `Ok n`, o valor está em `n` e a action continua; se for `Err e`, a action aborta imediatamente e retorna `Err e`. É o que `|` faz, mas em vez de usar um fallback, propaga o erro.

`ler_palpite` retorna `Result::(Int, Text)` — sucesso traz o número, erro traz a mensagem. O `?` permite que o corpo da action seja linear: ler input, converter, devolver `Ok n`. Sem `match`, sem aninhamento.

Com essas duas funções, o `jogar` fica raso:

```kata
action jogar (alvo::Int) => Unit
    loop
        match ler_palpite!()
            Ok palpite:
                match comparar palpite alvo
                    Some msg: echo!(msg)
                    None:
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

Dois níveis de `match`, cada um com um propósito: o primeiro decide se o input foi válido, o segundo decide o que fazer com a dica. Sem aninhamento de `Boolean` dentro de `Boolean` dentro de `Result`.

O código completo:

```kata
action ler_palpite => Result::(Int, Text)
    let n := int!(input!("Palpite: ")) ?
    Ok n

comparar :: Int Int => Optional::Text
lambda palpite alvo:
    > palpite alvo: Some "muito alto"
    < palpite alvo: Some "muito baixo"
    otherwise: None

action jogar (alvo::Int) => Unit
    loop
        match ler_palpite!()
            Ok palpite:
                match comparar palpite alvo
                    Some msg: echo!(msg)
                    None:
                        echo!("acertou!")
                        break
            Err e: echo!("não é um número")

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

O output acima é de uma partida onde o alvo era 42. Como o número é aleatório, a sequência de dicas muda a cada execução — o que torna o jogo rejogável. Tente algumas vezes.

### Por que decompor?

A versão decomposta é maior — mais linhas, mais definições. Mas cada peça faz uma coisa só:

- `comparar` é pura — dá para testar isoladamente, sem mockar stdin. Dá para reusar num modo de jogo diferente (ex: dois jogadores).
- `ler_palpite` encapsula I/O + conversão — o `?` aplaina o que seria um `match` inteiro.
- `jogar` orquestra — lê, compara, reage. Cada `match` tem um nível.

A versão aninhada (segunda) não é "errada" — é o ponto de chegada natural quando você está aprendendo. A decomposição é o próximo passo: quando o aninhamento começa a atrapalhar a leitura, é hora de extrair.

## O que você aprendeu

Você construiu um jogo interativo completo. No caminho, tocou em:

- **Actions** — funções impuras com `!` (`rand_int!`, `input!`, `int!`, `echo!`)
- **Funções puras** — sem `!`, sem efeitos colaterais (`comparar`)
- **`Result`** — success ou erro, sempre via `match`
- **`Optional`** — presença ou ausência de valor (`Some` / `None`)
- **`|`** — operador de fallback: desempacota `Ok`, usa o lado direito se `Err`
- **`?`** — operador de fail-fast: desempacota `Ok`, propaga `Err` se falhar
- **`match`** — condicional sem `if`, examinando a forma do valor
- **`loop` e `break`** — iteração e saída de loop

Cada um desses conceitos é coberto em profundidade nos próximos capítulos. O capítulo 3 mostra a sintaxe básica. O capítulo 7 explica actions, `var`, e `loop`. O capítulo 6 cobre `match` em detalhe. O capítulo 12 mostra I/O completo — `open!`, `readline!`, e o módulo `stdio`.

Por enquanto, você já escreveu um programa real em Kata — em três versões, da mais simples à mais decomposta. Isso é mais do que a maioria das linguagens oferece no primeiro dia.