mod cli;
mod config;
mod copy;
mod dispatch;
mod hash;
mod log;
mod manifest;
mod matcher;
mod mirror;
mod move_files;
mod notify;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_args(args.into_iter()) {
        Ok(options) => {
            let code = dispatch::run(&options);
            std::process::exit(code);
        }
        Err(code) => {
            eprintln!("{}", cli::USAGE);
            std::process::exit(code);
        }
    }
}
