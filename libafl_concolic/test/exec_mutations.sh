#!/bin/bash
set -eux;

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"
cd "$SCRIPT_DIR"

# this test intends to ...
# 1. compile symcc with the rust/tracing backend
# 2. compile a program using this symcc
# 3. run the program, capturing constraints
# 4. print the constraints in human readable form for verification
# 5. check that the captured constraints match those that we expect

# clone symcc
if [ ! -d "symcc" ]; then
    echo "cloning symcc"
    git clone https://github.com/sfu-rsl/symcc.git symcc
    cd symcc
    git checkout 96cab0e3d1159b2d74e7f2e69479fa0e44fcdd8a
    cd ..
fi

if [ ! -d "symcc_build" ]; then
    echo "building symcc"
    mkdir symcc_build
    cd symcc_build
    cmake -G Ninja -DZ3_TRUST_SYSTEM_VERSION=on ../symcc 
    ninja
    cd ..
fi


echo "building runtime"
cargo build -p runtime_exec

echo "building dump_constraints_mutations"
cargo build -p dump_constraints_mutations

echo "building target"
SYMCC_RUNTIME_DIR=../../target/debug symcc_build/symcc symcc/test/if.c -o "if"

echo "running target with dump_constraints_mutations"
cargo run -p dump_constraints_mutations -- --plain-text --output constraints.txt -- ./if < if_test_input

# echo "constraints: "
# cat constraints.txt

# # site_id's in the constraints trace will differ for every run. we therefore filter those.
# sed 's/, location: .* / /' < constraints.txt > constraints_filtered.txt
# sed 's/, location: .* / /' < expected_constraints.txt > expected_constraints_filtered.txt

# diff constraints_filtered.txt expected_constraints_filtered.txt
