#[tokio::main]
async fn main() {
    let command = match hyprlayer_server::Command::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    std::process::exit(hyprlayer_server::run(command).await);
}
