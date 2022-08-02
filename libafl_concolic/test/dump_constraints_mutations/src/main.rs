//! This is a straight-forward command line utility that can dump constraints written by a tracing runtime.
//! It achieves this by running an instrumented target program with the necessary environment variables set.
//! When the program has finished executing, it dumps the traced constraints to a file.

use clap::{self, StructOpt};
use std::{
    ffi::OsString,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    process::{exit, Command},
    string::ToString,
};

use libafl::{
    bolts::{
        shmem::{ShMem, ShMemProvider, StdShMemProvider},
        AsSlice,
    },
    observers::concolic::{
        serialization_format::{MessageFileReader, MessageFileWriter, DEFAULT_ENV_NAME},
        EXPRESSION_PRUNING, HITMAP_ENV_NAME, NO_FLOAT_ENV_NAME, SELECTIVE_SYMBOLICATION_ENV_NAME,
        ConcolicObserver, ConcolicMetadata
    },

    stages::concolic::generate_mutations, prelude::AsMutSlice,
};

#[derive(Debug, StructOpt)]
#[clap(
    name = "dump_constraints",
    about = "Dump tool for concolic constraints."
)]
struct Opt {
    /// Outputs plain text instead of binary
    #[clap(short, long)]
    plain_text: bool,

    /// Outputs coverage information to the given file
    #[clap(short, long)]
    coverage_file: Option<PathBuf>,

    /// Symbolizes only the given input file offsets.
    #[clap(short, long)]
    symbolize_offsets: Option<Vec<usize>>,

    /// Concretize all floating point operations.
    #[clap(long)]
    no_float: bool,

    /// Prune expressions from high-frequency code locations.
    #[clap(long)]
    prune: bool,

    /// Trace file path, "trace" by default.
    #[clap(parse(from_os_str), short, long)]
    output: Option<PathBuf>,

    /// Target program and arguments
    #[clap(last = true)]
    program: Vec<OsString>,
}

fn main() {
    const COVERAGE_MAP_SIZE: usize = 65536;

    let opt = Opt::parse();

    let mut shmemprovider = StdShMemProvider::default();
    let concolic_shmem = shmemprovider
        .new_shmem(1024 * 1024 * 1024)
        .expect("unable to create shared mapping");
    concolic_shmem
        .write_to_env(DEFAULT_ENV_NAME)
        .expect("unable to write shared mapping info to environment");

    let coverage_map = StdShMemProvider::new()
        .unwrap()
        .new_shmem(COVERAGE_MAP_SIZE)
        .unwrap();
    //let the forkserver know the shmid
    coverage_map.write_to_env(HITMAP_ENV_NAME).unwrap();

    if let Some(symbolize_offsets) = opt.symbolize_offsets {
        std::env::set_var(
            SELECTIVE_SYMBOLICATION_ENV_NAME,
            symbolize_offsets
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    }

    if opt.no_float {
        std::env::set_var(NO_FLOAT_ENV_NAME, "1");
    }

    if opt.prune {
        std::env::set_var(EXPRESSION_PRUNING, "1");
    }

    let res = Command::new(&opt.program.first().expect("no program argument given"))
        .args(opt.program.iter().skip(1))
        .status()
        .expect("failed to spawn program");
    {
        if let Some(coverage_file_path) = opt.coverage_file {
            let mut f = BufWriter::new(
                File::create(coverage_file_path).expect("unable to open coverage file"),
            );
            for (index, count) in coverage_map
                .as_slice()
                .iter()
                .enumerate()
                .filter(|(_, &v)| v != 0)
            {
                writeln!(f, "{}\t{}", index, count).expect("failed to write coverage file");
            }
        }

        // open a new scope to ensure our resources get dropped before the exit call at the end
        let output_file_path = opt.output.unwrap_or_else(|| "trace".into());
        let mut output_file =
            BufWriter::new(File::create(output_file_path).expect("unable to open output file"));

        // reader
        let observer = ConcolicObserver::new("concolic".to_string(), concolic_shmem.as_slice());
        let metadata = observer.create_metadata_from_current_map();
        
        // let mut reader = MessageFileReader::from_length_prefixed_buffer(concolic_shmem.as_slice())
            // .expect("unable to create trace reader");
        if opt.plain_text {
            // while let Some(message) = reader.next_message() {
            //     if let Ok((id, message)) = message {
            //         writeln!(output_file, "{}\t{:?}", id, message)
            //             .expect("failed to write to output file");
            //     } else {
            //         break;
            //     }
            // }
            println!("Printing mutations...");


            // hardcoded
            let input = b"1234";
            println!("Original Input:\n{:?}\n", input);
            println!("Original Input as i32: {}\n", i32::from_le_bytes(*input));
            let mutations = generate_mutations(metadata.iter_messages());


            for (id, mutation) in mutations.iter().enumerate() {
                let mut new_input = input.to_vec();

                for (idx, byte) in mutation {
                    new_input[*idx] = *byte;
                }

                println!("Mutation #{}:", id);
                println!("{:?} ({})", new_input, i32::from_le_bytes(new_input.as_slice().try_into().expect("Failed to convert mutation to i32!")));
            }

        } else {
            unimplemented!("Non plain-text option not handled!");
            // let mut writer =
            //     MessageFileWriter::from_writer(output_file).expect("unable to create trace writer");
            // while let Some(message) = reader.next_message() {
            //     if let Ok((_, message)) = message {
            //         writer
            //             .write_message(message)
            //             .expect("unable to write message");
            //     } else {
            //         break;
            //     }
            // }
        }
    }

    exit(res.code().expect("failed to get exit code from program"));
}
