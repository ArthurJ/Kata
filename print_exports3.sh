sed -i 's/new_exports.extend(methods.clone());/new_exports.extend(methods.clone());\n                println!("TYPE METHODS FOR {}: {:?}", exp, methods);/' src/type_checker/environment.rs
cargo run -- build examples/mock_math.kata
sed -i 's/println!("TYPE METHODS FOR {}: {:?}", exp, methods);//' src/type_checker/environment.rs
