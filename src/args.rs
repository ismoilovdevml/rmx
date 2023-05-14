// args.rs

use std::env;

pub fn parse_args() -> Vec<String> {
    env::args().collect()
}
