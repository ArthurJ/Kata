# Capítulo 13 — Módulos

Kata organiza código em módulos. Cada arquivo `.kata` é um módulo. Itens exportados com `export` ficam visíveis para quem importar; itens não exportados são privados.

## Exportando

Declare funções, actions, e tipos normalmente. No final, `export` lista o que é público:

```kata
# mod_math.kata
dobrar :: Int => Int
lambda x: + x x

triplicar :: Int => Int
lambda x: * x 3

quadrupar :: Int => Int
lambda x: * x 4

export dobrar triplicar
```

`quadrupar` não está na lista — é privada do módulo.

## Importando módulo inteiro

`import mod` traz o módulo para o escopo. Acesso via prefixo `mod.fn`:

```kata
import mod_math

action main
    let dobro := mod_math.dobrar 21
    let triplo := mod_math.triplicar 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Import seletivo

`import mod.(item)` traz itens específicos para o escopo direto, sem prefixo:

```kata
import mod_math.(triplicar)

action main
    let triplo := triplicar 21
    echo!(triplo)
main!()
```

```
63
```

## Import com alias

Renomeie itens ao importar para evitar colisões:

```kata
import mod_math.(dobrar as d, triplicar as t)

action main
    let dobro := d 21
    let triplo := t 21
    echo!(dobro)
    echo!(triplo)
main!()
```

```
42
63
```

## Resolução de paths

O loader procura módulos em dois lugares:

1. **Diretório do arquivo importador** (`entry_dir`) — onde está o arquivo que faz o `import`.
2. **Stdlib** — biblioteca padrão, como fallback.

Para `import mod_math`, o loader procura `mod_math.kata` no diretório do importador. Paths aninhados (`import subdir.mod`) seguem a estrutura de diretórios.

## `mod.kata` — diretório como módulo

Um diretório pode ser importado como unidade se contiver `mod.kata`:

```
projeto/
  main.kata        → import math.(dobrar)
  math/
    mod.kata       → dobrar :: Int => Int ...
    algebra.kata   → ...
```

```kata
# math/mod.kata
dobrar :: Int => Int
lambda x: * x 2
export dobrar
```

```kata
# main.kata
import math.(dobrar)

dobrar 21
```

```
42
```

Sem `mod.kata`, `import math` é erro. Mas `import math.algebra` funciona sem `mod.kata` — submódulos diretos não precisam dele.

## `super.` — importando de diretórios pai

`super.` sobe um nível na árvore de diretórios, relativo ao arquivo que faz o import:

```
projeto/
  utils.kata       → helper :: Int => Int ...
  math/
    algebra.kata   → import super.utils.(helper)
```

```kata
# utils.kata
helper :: Int => Int
lambda x: + x 1
export helper
```

```kata
# math/algebra.kata
import super.utils.(helper)

helper 41
```

```
42
```

`super.super.X` sobe dois níveis. `super` só resolve no diretório resolvido — sem fallback para stdlib.

## `stdlib.` — forçando a biblioteca padrão

Quando há um módulo local com o mesmo nome da stdlib, `stdlib.` força a stdlib:

```kata
import stdlib.math.(pi)

pi
```

```
3.141592653589793
```

Sem o prefixo `stdlib.`, `import math` carregaria o módulo local (se existir). Com `stdlib.`, ignora o local e vai direto para a stdlib.

## Próximo capítulo

Módulos organizam código. O próximo capítulo mostra as otimizações que o compilador aplica automaticamente — TCO, TRMA, stream fusion, e `@cache`. → [Capítulo 14](14-otimizacoes.md)