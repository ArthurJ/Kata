sed -i 's/let mut new_exports = Vec::new();/let mut new_exports = Vec::new();\n        println!("EXPANDING: {:?}", self.exports);/' src/type_checker/environment.rs
cargo run -- build examples/mock_math.kata
sed -i 's/println!("EXPANDING: {:?}", self.exports);//' src/type_checker/environment.rs
