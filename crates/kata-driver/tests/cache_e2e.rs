// This file was split into focused test modules:
//   cache_basic_e2e.rs       — basic single-clause, cache hit, multi-clause, guards, tail call
//   cache_types_e2e.rs       — Float, Text, Struct, List cache key tests
//   cache_strategies_e2e.rs — FIFO, MRU, LFU tests + eviction
//   cache_config_e2e.rs      — capacity, no-args, empty dict, capacity zero error
//   cache_tco_e2e.rs         — TCO large n, mixed tail/non-tail, timer+TCO
