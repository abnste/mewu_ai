// SPDX-License-Identifier: MPL-2.0
fn main() {
    if mewu_extension::run_stdio_worker().is_err() {
        // stdout belongs exclusively to the protocol; never print plugin data.
        std::process::exit(1);
    }
}
