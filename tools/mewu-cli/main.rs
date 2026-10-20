// SPDX-License-Identifier: MPL-2.0
mod cli;

fn main() {
    std::process::exit(cli::run(std::env::args_os().collect()));
}
