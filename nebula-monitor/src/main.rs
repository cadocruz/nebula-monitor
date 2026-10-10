fn main() -> Result<(), Box<dyn std::error::Error>> {
    nebula_monitor::run(nebula_monitor::registry::builtin())
}
