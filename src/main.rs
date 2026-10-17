use wow_vanilla_server::server;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    server::main(server::Start::Server).into()
}
