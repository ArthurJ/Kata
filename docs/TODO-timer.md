# TODO — `@timer` diretiva

## Ideia

Diretiva que mede o tempo de execução de uma função/action e publica via `@log`.

## Sintaxe proposta

```kata
@timer{topic: "stdout"}
dobro :: Int => Int
lambda n: * n 2
```

## Comportamento

- **Prólogo:** captura timestamp inicial (antes do `@log` Enter e do `@cache` lookup)
- **Epílogo:** captura timestamp final, computa delta, publica via `kata_rt_log_publish`
- **Cache hit:** o hit block faz `return_` direto — timer não dispara no epílogo.
  O usuário veria tempo 0 ou ausente. Decidir: (a) aceitar como limitação,
  (b) mover o hit para o epílogo também, ou (c) publicar delta ~0 do hit.

## Interação com outras diretivas

Ordem proposta no codegen de Sig:

```
1. bind_patterns_to_params
2. @timer start        ← antes de tudo
3. @log Enter
4. @cache lookup
5. [hit → jump epílogo]
6. [miss → body]
7. epilogue_block:
   a. @log Exit
   b. @cache insert
   c. @timer stop + publish
   d. return_(result)
```

## Implementação

- Runtime: `kata_rt_timer_now() -> i64` (monotonic clock, nanossegundos)
- Codegen: `iconst` do timestamp inicial no prólogo, subtração no epílogo
- FFI: registrar `kata_rt_timer_now` no `ffi_registry.rs`
- Tópico default: `"stdout"` (delta formatado como `duracao: Xms`)

## Estado

Não implementado. Criado durante Fase 5 do Fio 12 como ideia para o futuro.