sed -i 's/for exp in new_exports {/for exp in new_exports.clone() { println!("INSERTING EXPORT: {}", exp); /' src/type_checker/environment.rs
cargo run -- build examples/mock_math.kata > exp_debug.txt 2>&1
sed -i 's/for exp in new_exports.clone() { println!("INSERTING EXPORT: {}", exp); /for exp in new_exports {/' src/type_checker/environment.rs
cat exp_debug.txt | grep "INSERTING EXPORT"
