use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use kata_lsp::analysis::run_frontend;

fn bench_frontend(c: &mut Criterion) {
    let mut group = c.benchmark_group("frontend");

    // Arquivo real: stdlib/core.kata (619 linhas)
    let core_source = include_str!("../../../stdlib/core.kata");
    group.throughput(Throughput::Elements(core_source.lines().count() as u64));
    group.bench_with_input(
        BenchmarkId::new("real", "stdlib/core.kata (619 lines)"),
        &core_source,
        |bench, source| {
            bench.iter(|| {
                let _ = run_frontend(source, None);
            });
        },
    );

    // Arquivo real: examples/refined_types.kata (73 linhas)
    let refined_source = include_str!("../../../examples/refined_types.kata");
    group.throughput(Throughput::Elements(refined_source.lines().count() as u64));
    group.bench_with_input(
        BenchmarkId::new("real", "refined_types.kata (73 lines)"),
        &refined_source,
        |bench, source| {
            bench.iter(|| {
                let _ = run_frontend(source, None);
            });
        },
    );

    // Arquivo sintético — gera N linhas de código Kata para medir escalabilidade
    for &n in &[50, 200, 500, 1000] {
        let synthetic = generate_synthetic_kata(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(
            BenchmarkId::new("synthetic", format!("{n} lines")),
            &synthetic,
            |bench, source| {
                bench.iter(|| {
                    let _ = run_frontend(source, None);
                });
            },
        );
    }

    group.finish();
}

/// Gera código Kata sintético com N linhas.
///
/// Mistura declarações `let`, actions, funções, e expressões para
/// exercitar todas as camadas do front-end (lex, parse, resolve, infer).
fn generate_synthetic_kata(n: usize) -> String {
    let mut out = String::with_capacity(n * 60);

    for i in 0..n {
        match i % 4 {
            0 => {
                // let binding com expressão aritmética
                out.push_str(&format!("constant x{i} := + {i} (* {i} 2)\n"));
            }
            1 => {
                // action com body simples
                out.push_str(&format!("action act{i}(a Int, b Int) Int {{ + a b }}\n"));
            }
            2 => {
                // function com cláusula única
                out.push_str(&format!(
                    "fn f{i}(n Int) Int := match n {{ 0 => 1, _ => * n (f{i} (- n 1)) }}\n"
                ));
            }
            3 => {
                // let com list literal
                out.push_str(&format!(
                    "constant lst{i} := [{}]\n",
                    vec![i.to_string(); 5].join(", ")
                ));
            }
            _ => unreachable!(),
        }
    }

    // Entry point para que o módulo não seja vazio
    out.push_str("constant main := 0\n");
    out
}

criterion_group!(benches, bench_frontend);
criterion_main!(benches);
