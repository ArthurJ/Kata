rustc --edition 2021 src/main.rs -o print_ast --extern kata=target/debug/libkata.rlib -L dependency=target/debug/deps
./print_ast
