//! Config command handler

use nu_analytics::config::Config;

/// Handle the config subcommand
///
/// If a key is provided, print only that config value.
/// If no key is provided, print all configuration values.
pub fn handle_config_command(config_defaults: &str, key: Option<String>) {
    let config = Config::from_defaults(config_defaults);

    if let Some(k) = key {
        // Print specific config value
        match k.as_str() {
            // Logging section
            "level" => println!("{}", config.logging.level),
            "file" => println!("{}", config.logging.file),
            "verbose" => println!("{}", config.logging.verbose),

            // Database section
            "token" => println!("{}", config.database.token),
            "endpoint" => println!("{}", config.database.endpoint),

            // Paths section
            "plans_dir" => println!("{}", config.paths.plans_dir),
            "out_dir" => println!("{}", config.paths.out_dir),

            // Unknown key
            _ => eprintln!("Unknown config key: '{k}'"),
        }
    } else {
        // Print all config values
        println!("\n=== Configuration ===\n");

        println!("[Logging]");
        println!("  level = \"{}\"", config.logging.level);
        println!("  file = \"{}\"", config.logging.file);
        println!("  verbose = {}", config.logging.verbose);

        println!("\n[Database]");
        println!("  token = \"{}\"", config.database.token);
        println!("  endpoint = \"{}\"", config.database.endpoint);

        println!("\n[Paths]");
        println!("  plans_dir = \"{}\"", config.paths.plans_dir);
        println!("  out_dir = \"{}\"", config.paths.out_dir);
        println!();
    }
}
