sed -i 's/if let Some(variants) = self.env.enums.get(&type_name) {/println!("DEBUG CALL: name={}, type_name={}", name, type_name);\n                if let Some(variants) = self.env.enums.get(\&type_name) {/' src/codegen/expr.rs
cargo run -- build examples/test_enum.kata
sed -i 's/println!("DEBUG CALL: name={}, type_name={}", name, type_name);//' src/codegen/expr.rs
