mod cli;
mod config;
mod copy;
mod hash;
mod manifest;
mod matcher;
mod mirror;
mod move_files;
mod notify;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_args(args.into_iter()) {
        Ok(_) => {
            println!("{}", cli::USAGE);
            std::process::exit(0);
        }
        Err(code) => {
            eprintln!("{}", cli::USAGE);
            std::process::exit(code);
        }
    }
}
