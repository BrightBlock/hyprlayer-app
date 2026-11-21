#[cfg(target_os = "macos")]
mod large_blocks;

#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: large_blocks::LargeBlocks = large_blocks::LargeBlocks;

#[tokio::main]
async fn main() {
    // glibc raises its mmap threshold to each large block freed, after which a transfer's
    // buffers stay in arenas for the process's life; pinned, they are unmapped once freed.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
    }
    let command = match hyprlayer_server::Command::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    std::process::exit(hyprlayer_server::run(command).await);
}
