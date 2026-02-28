use std::io::{self, Write};
use std::net::Ipv4Addr;

use anyhow::{Context, Result};

use shared::data::{ClientConfig, Sherpa};
use shared::util::{create_client_config, create_dir, file_exists, term_msg_surround};

pub fn init(sherpa: &Sherpa, force: bool) -> Result<()> {
    term_msg_surround("Sherpa Client Init");

    if file_exists(&sherpa.config_file_path) && !force {
        println!(
            "Config file already exists: {}\nUse --force to overwrite.",
            sherpa.config_file_path
        );
        return Ok(());
    }

    // Prompt for server IP
    let server_ip = prompt_with_default("Server IP address", "127.0.0.1")?;
    let server_ipv4: Ipv4Addr = server_ip
        .parse()
        .context("Invalid IPv4 address. Expected format: x.x.x.x")?;

    // Prompt for server port
    let port_str = prompt_with_default("Server port", "3030")?;
    let server_port: u16 = port_str
        .parse()
        .context("Invalid port number. Expected a number between 1 and 65535")?;

    // Create config directory if needed
    create_dir(&sherpa.config_dir)?;

    // Build and write client config
    let config = ClientConfig {
        server_ipv4,
        server_port,
        ..ClientConfig::default()
    };

    create_client_config(&config, &sherpa.config_file_path)?;

    println!("Config written to: {}", sherpa.config_file_path);

    Ok(())
}

fn prompt_with_default(prompt: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read input")?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}
