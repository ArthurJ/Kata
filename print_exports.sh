sed -i 's/let all_exports: Vec<(String, Option<String>)> = env_clone.exports.iter().map(|e| (e.clone(), None)).collect();/let all_exports: Vec<(String, Option<String>)> = env_clone.exports.iter().map(|e| (e.clone(), None)).collect();\nprintln!("PRELUDE EXPORTS: {:?}", all_exports);/' src/type_checker/checker.rs
cargo run -- build examples/mock_math.kata
sed -i 's/println!("PRELUDE EXPORTS: {:?}", all_exports);//' src/type_checker/checker.rs
