//! A libfuzzer-like fuzzer with llmp-multithreading support and restarts
//! The example harness is built for `stb_image`.
use mimalloc::MiMalloc;
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use std::{
    env,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use clap::{self, Parser};
use libafl::{
    bolts::{
        current_nanos,
        rands::StdRand,
        shmem::{ShMem, ShMemProvider, StdShMemProvider},
        tuples::{tuple_list, Named},
        AsMutSlice, AsSlice,
    },
    corpus::{Corpus, InMemoryCorpus, OnDiskCorpus},
    events::{setup_restarting_mgr_std, EventConfig},
    executors::{
        command::CommandConfigurator, inprocess::InProcessExecutor, ExitKind, ShadowExecutor,
    },
    feedback_or,
    feedbacks::{CrashFeedback, MaxMapFeedback, TimeFeedback},
    fuzzer::{Fuzzer, StdFuzzer},
    inputs::{BytesInput, HasTargetBytes, Input},
    monitors::MultiMonitor,
    mutators::{
        scheduled::{havoc_mutations, StdScheduledMutator},
        token_mutations::I2SRandReplace,
    },
    observers::{
        concolic::{
            serialization_format::{DEFAULT_ENV_NAME, DEFAULT_SIZE},
            ConcolicObserver,
        },
        TimeObserver,
    },
    prelude::{Executor, ShMemId, SimpleEventManager, SimpleMonitor, StdMapObserver},
    schedulers::{IndexesLenTimeMinimizerScheduler, QueueScheduler},
    stages::{
        ConcolicTracingStage, ShadowTracingStage, SimpleConcolicMutationalStage,
        StdMutationalStage, TracingStage,
    },
    state::{HasCorpus, StdState},
    Error,
};
use libafl_targets::{
    edges_max_num, libfuzzer_initialize, libfuzzer_test_one_input, std_edges_map_observer,
    CmpLogObserver,
};

#[derive(Debug, Parser)]
struct Opt {
    /// This node should do concolic tracing + solving instead of traditional fuzzing
    #[arg(short, long)]
    concolic: bool,
}

pub fn main() {
    // Registry the metadata types used in this fuzzer
    // Needed only on no_std
    //RegistryBuilder::register::<Tokens>();

    let opt = Opt::parse();

    println!(
        "Workdir: {:?}",
        env::current_dir().unwrap().to_string_lossy().to_string()
    );
    fuzz(
        &[PathBuf::from("./corpus")],
        PathBuf::from("./crashes"),
        1337,
        opt.concolic,
    )
    .expect("An error occurred while fuzzing");
}

/// The actual fuzzer
fn fuzz(
    corpus_dirs: &[PathBuf],
    objective_dir: PathBuf,
    broker_port: u16,
    concolic: bool,
) -> Result<(), Error> {
    // 'While the stats are state, they are usually used in the broker - which is likely never restarted
    let monitor = MultiMonitor::new(|s| println!("{s}"));

    // The restarting state will spawn the same process again as child, then restarted it each time it crashes.
    let (state, mut restarting_mgr) =
        match setup_restarting_mgr_std(monitor, broker_port, EventConfig::from_name("default")) {
            Ok(res) => res,
            Err(err) => match err {
                Error::ShuttingDown => {
                    return Ok(());
                }
                _ => {
                    panic!("Failed to setup the restarter: {err}");
                }
            },
        };

    // Create an observation channel using the coverage map
    // We don't use the hitcounts (see the Cargo.toml, we use pcguard_edges)
    let mut shmem_provider = StdShMemProvider::new().unwrap();
    let mut edges = shmem_provider.new_shmem(edges_max_num()).unwrap();
    let edges_shmem_id = edges.id();
    let edges_shmem_size = edges.len();
    let edges_observer = unsafe { StdMapObserver::new("edges", edges.as_mut_slice()) };

    // Create an observation channel to keep track of the execution time
    let time_observer = TimeObserver::new("time");

    let cmplog_observer = CmpLogObserver::new("cmplog", true);

    // Feedback to rate the interestingness of an input
    // This one is composed by two Feedbacks in OR
    let mut feedback = feedback_or!(
        // New maximization map feedback linked to the edges observer and the feedback state
        MaxMapFeedback::tracking(&edges_observer, true, false),
        // Time feedback, this one does not need a feedback state
        TimeFeedback::with_observer(&time_observer)
    );

    // A feedback to choose if an input is a solution or not
    let mut objective = CrashFeedback::new();

    // If not restarting, create a State from scratch
    let mut state = state.unwrap_or_else(|| -> StdState<BytesInput, InMemoryCorpus<BytesInput>, libafl::prelude::RomuDuoJrRand, OnDiskCorpus<BytesInput>> {
        StdState::new(
            // RNG
            StdRand::with_seed(current_nanos()),
            // Corpus that will be evolved, we keep it in memory for performance
            InMemoryCorpus::new(),
            // Corpus in which we store solutions (crashes in this example),
            // on disk so the user can get them after stopping the fuzzer
            OnDiskCorpus::new(objective_dir).unwrap(),
            // States of the feedbacks.
            // The feedbacks can report the data that should persist in the State.
            &mut feedback,
            // Same for objective feedbacks
            &mut objective,
        )
        .unwrap()
    });

    println!("We're a client, let's fuzz :)");

    // A minimization+queue policy to get testcasess from the corpus
    let scheduler = IndexesLenTimeMinimizerScheduler::new(QueueScheduler::new());

    // A fuzzer with feedbacks and a corpus scheduler
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    // Create the executor for an in-process function with just one observer for edge coverage
    let mut executor = ShadowExecutor::new(
        CoverageConfigurator {
            edges_shmem_id,
            edges_shmem_size,
        }
        .into_executor(tuple_list!(edges_observer, time_observer)),
        tuple_list!(cmplog_observer),
    );

    // In case the corpus is empty (on first run), reset
    if state.must_load_initial_inputs() {
        state
            .load_initial_inputs(&mut fuzzer, &mut executor, &mut restarting_mgr, corpus_dirs)
            .unwrap_or_else(|_| panic!("Failed to load initial corpus at {corpus_dirs:?}"));
        println!("We imported {} inputs from disk.", state.corpus().count());
    }

    if concolic {
        println!("We're doing concolic fuzzing!");

        // The shared memory for the concolic runtime to write its trace to
        let mut concolic_shmem = shmem_provider.new_shmem(DEFAULT_SIZE).unwrap();
        concolic_shmem.write_to_env(DEFAULT_ENV_NAME).unwrap();

        // The concolic observer observers the concolic shared memory map.
        let concolic_observer =
            ConcolicObserver::new("concolic".to_string(), concolic_shmem.as_mut_slice());

        let concolic_observer_name = concolic_observer.name().to_string();

        let concolic_executor =
            ConcolicConfigurator::default().into_executor(tuple_list!(concolic_observer));
        let tracing = TracingStage::new(concolic_executor);
        // The order of the stages matter!
        let mut stages = tuple_list!(
            // Create a concolic trace
            ConcolicTracingStage::new(tracing, concolic_observer_name),
            // Use the concolic trace for z3-based solving
            SimpleConcolicMutationalStage::default(),
        );

        fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut restarting_mgr)?;
    } else {
        // Setup a randomic Input2State stage
        let i2s =
            StdMutationalStage::new(StdScheduledMutator::new(tuple_list!(I2SRandReplace::new())));

        // Setup a basic mutator
        let mutator = StdScheduledMutator::new(havoc_mutations());
        let mutational = StdMutationalStage::new(mutator);

        // Setup a tracing stage in which we log comparisons
        let tracing = ShadowTracingStage::new(&mut executor);

        // The order of the stages matter!
        let mut stages = tuple_list!(tracing, i2s, mutational);

        fuzzer.fuzz_loop(&mut stages, &mut executor, &mut state, &mut restarting_mgr)?;
    }

    // Never reached
    Ok(())
}

#[derive(Debug)]
struct CoverageConfigurator {
    edges_shmem_id: ShMemId,
    edges_shmem_size: usize,
}

impl CommandConfigurator for CoverageConfigurator {
    fn spawn_child<I: Input + HasTargetBytes>(&mut self, input: &I) -> Result<Child, Error> {
        /* IMPORTANT NOTE: As the rate of execution is high for this mode,
         * and it is writing the input to the file each time, it may cause
         * serious exhaustion of the underlying storage.
         * This is meant for a proof of concept and in a practical setting you
         * should use another way such as a memory-mapped file.
         * Also, in this example, writing to STDIN didn't work.
         */

        // We use the shmem id as a distinguishing factor for different clients.
        let input_path = format!("cur_input{}", self.edges_shmem_id);
        input.to_file(input_path.as_str())?;
        Ok(Command::new("./target_symcc.out")
            .arg(input_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("EDGES_MAP_SHMEM", self.edges_shmem_id.to_string())
            .env("EDGES_MAP_SHMEM_SIZE", self.edges_shmem_size.to_string())
            .env("SYMCC_NO_SYMBOLIC_INPUT", "1")
            .spawn()
            .expect("failed to start process"))
    }
}

#[derive(Default, Debug)]
pub struct ConcolicConfigurator;

impl CommandConfigurator for ConcolicConfigurator {
    fn spawn_child<I: Input + HasTargetBytes>(&mut self, input: &I) -> Result<Child, Error> {
        let input_path = "cur_input_concolic";
        input.to_file(input_path)?;

        Ok(Command::new("./target_symcc.out")
            .arg(input_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("SYMCC_INPUT_FILE", input_path)
            .env("SYMCC_NO_SYMBOLIC_INPUT", "0")
            .spawn()
            .expect("failed to start process"))
    }
}
