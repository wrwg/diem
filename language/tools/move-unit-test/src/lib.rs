// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

pub mod test_reporter;
pub mod test_runner;
use crate::test_runner::TestRunner;
use move_core_types::language_storage::ModuleId;
use move_lang::{
    errors,
    shared::{CompilationEnv, Flags},
    unit_test::{self, TestPlan},
    Pass, PassResult,
};
use std::{
    io::{Result, Write},
    marker::Send,
    sync::Mutex,
};
use structopt::*;

#[derive(Debug, StructOpt, Clone)]
#[structopt(name = "Move Unit Test", about = "Unit testing for Move code.")]
pub struct UnitTestingConfig {
    /// Bound the number of instructions that can be executed by any one test.
    #[structopt(
        name = "instructions",
        default_value = "5000",
        short = "i",
        long = "instructions"
    )]
    pub instruction_execution_bound: u64,

    /// A filter string to determine which unit tests to run
    #[structopt(name = "filter", short = "f", long = "filter")]
    pub filter: Option<String>,

    /// Number of threads to use for running tests.
    #[structopt(
        name = "num_threads",
        default_value = "8",
        short = "t",
        long = "threads"
    )]
    pub num_threads: usize,

    /// Report test statistics at the end of testing
    #[structopt(name = "report_statistics", short = "s", long = "statistics")]
    pub report_statistics: bool,

    /// Show the storage state at the end of execution of a failing test
    #[structopt(name = "global_state_on_error", short = "g", long = "state_on_error")]
    pub report_storage_on_error: bool,

    /// Source files
    #[structopt(name = "sources")]
    pub source_files: Vec<String>,

    /// Use the stackless bytecode interpreter to run the tests and cross check its results with
    /// the execution result from Move VM.
    #[structopt(long = "stackless")]
    pub check_stackless_vm: bool,

    /// Verbose mode
    #[structopt(short = "v", long = "verbose")]
    pub verbose: bool,
}

fn format_module_id(module_id: &ModuleId) -> String {
    format!(
        "0x{}::{}",
        module_id.address().short_str_lossless(),
        module_id.name()
    )
}

impl UnitTestingConfig {
    /// Build a test plan from a unit test config
    pub fn build_test_plan(&self) -> Option<TestPlan> {
        let mut compilation_env = CompilationEnv::new(Flags::testing());
        let (files, pprog_and_comments_res) =
            move_lang::move_parse(&compilation_env, &self.source_files, &[], None).ok()?;
        let (_, pprog) = move_lang::unwrap_or_report_errors!(files, pprog_and_comments_res);
        let cfgir_result = move_lang::move_continue_up_to(
            &mut compilation_env,
            None,
            PassResult::Parser(pprog),
            Pass::CFGIR,
        );

        let (test_plan, cfgir) = match move_lang::unwrap_or_report_errors!(files, cfgir_result) {
            PassResult::CFGIR(cfgir) => (
                unit_test::plan_builder::construct_test_plan(&mut compilation_env, &cfgir),
                cfgir,
            ),
            _ => unreachable!(),
        };

        if let Err(errors) = compilation_env.check_errors() {
            errors::report_errors(files, errors);
        }

        let compilation_result = move_lang::move_continue_up_to(
            &mut compilation_env,
            None,
            PassResult::CFGIR(cfgir),
            Pass::Compilation,
        );

        let units = match move_lang::unwrap_or_report_errors!(files, compilation_result) {
            PassResult::Compilation(units) => units,
            _ => unreachable!(),
        };

        test_plan.map(|tests| TestPlan::new(tests, files, units))
    }

    /// Public entry point to Move unit testing as a library
    pub fn run_and_report_unit_tests<W: Write + Send>(
        &self,
        test_plan: TestPlan,
        writer: W,
    ) -> Result<W> {
        let shared_writer = Mutex::new(writer);

        writeln!(shared_writer.lock().unwrap(), "Running tests")?;
        let mut test_runner = TestRunner::new(
            self.instruction_execution_bound,
            self.num_threads,
            self.check_stackless_vm,
            self.verbose,
            self.report_storage_on_error,
            test_plan,
        )
        .unwrap();

        if let Some(filter_str) = &self.filter {
            test_runner.filter(filter_str)
        }

        let test_results = test_runner.run(&shared_writer).unwrap();
        if self.report_statistics {
            test_results.report_statistics(&shared_writer)?;
        }
        test_results.summarize(&shared_writer)?;

        let writer = shared_writer.into_inner().unwrap();
        Ok(writer)
    }
}
