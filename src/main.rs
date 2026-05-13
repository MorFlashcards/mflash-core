// src/main.rs

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use uuid::Uuid;

/// The headless, offline-first engine and CLI for mflash v4
#[derive(Parser)]
#[command(name = "mflash")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extracts an .mflash archive into the hidden local workspace cache
    Unpack {
        /// Path to the .mflash file (e.g., biology.mflash)
        input: PathBuf,
    },

    /// Compiles a local workspace into a compressed .mflash archive
    Pack {
        /// The UUID of the workspace to pack
        workspace_id: String,

        /// Output path (e.g., ./biology-updated.mflash)
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Inspects a workspace's deck.pb file
    Inspect {
        /// The UUID of the workspace to inspect
        workspace_id: String,
    },

    /// Compresses all media in a workspace using all available CPU cores
    Optimize {
        /// The UUID of the workspace to optimize
        workspace_id: String,
    },
}

fn main() {
    // Parse the terminal arguments
    let cli = Cli::parse();

    // Route the commands
    match &cli.command {
        Commands::Unpack { input } => {
            println!("🚀 Preparing to unpack: {}", input.display());

            // Generate a unique workspace ID
            let workspace_id = Uuid::new_v4().to_string();

            // Call the library unpacker.
            //
            // Note:
            // The package/crate name `mflash-core` is imported in Rust code as `mflash_core`.
            match mflash_core::workspace::unpack_deck(input, &workspace_id) {
                Ok(cache_path) => {
                    println!("✅ Successfully unpacked deck into workspace!");
                    println!("📁 Workspace ID: {}", workspace_id);
                    println!("📂 Path: {}", cache_path.display());

                    // 1. Initialize the SQLite Live Editor database.
                    let mut conn = mflash_core::db::init_workspace_db(&cache_path)
                        .expect("❌ Failed to initialize workspace database");

                    // 2. Try to read deck.pb, then sync it into SQLite.
                    if let Ok(deck) = mflash_core::schema::read_deck(&cache_path) {
                        mflash_core::db::import_pb_to_db(&mut conn, &deck)
                            .expect("❌ Failed to sync Protobuf to SQLite");

                        println!("✅ Synced deck.pb into workspace.db");
                    } else {
                        println!("ℹ️ No valid deck.pb found. Creating empty workspace.");
                    }
                }
                Err(e) => {
                    eprintln!("❌ Failed to unpack deck: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Pack {
            workspace_id,
            output,
        } => {
            println!(
                "📦 Packing workspace '{}' to {}",
                workspace_id,
                output.display()
            );

            let home = dirs::home_dir().expect("Could not find home directory");
            let cache_path = home.join(".mflash_cache").join(workspace_id);

            // 1. Connect to SQLite and export the live editor database to a Protobuf struct.
            let conn = mflash_core::db::init_workspace_db(&cache_path)
                .expect("❌ Failed to connect to workspace database");

            let deck = mflash_core::db::export_db_to_pb(&conn)
                .expect("❌ Failed to compile SQLite data");

            // 2. Write the Protobuf struct back to deck.pb.
            mflash_core::schema::write_deck(&cache_path, &deck)
                .expect("❌ Failed to write deck.pb");

            // 3. Zip the workspace into an .mflash archive.
            match mflash_core::workspace::pack_deck(workspace_id, output) {
                Ok(_) => {
                    println!("✅ Successfully packaged deck!");
                    println!("💾 Saved to: {}", output.display());
                }
                Err(e) => {
                    eprintln!("❌ Failed to pack deck: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Inspect { workspace_id } => {
            let home = dirs::home_dir().expect("Could not find home directory");
            let workspace_dir = home.join(".mflash_cache").join(&workspace_id);

            println!("🔍 Inspecting workspace: {}", workspace_id);

            // Read from the SQLite database to prove the live-state pipeline works.
            let conn = mflash_core::db::init_workspace_db(&workspace_dir)
                .expect("❌ Failed to connect to workspace database");

            match mflash_core::db::export_db_to_pb(&conn) {
                Ok(deck) => {
                    println!("\n✅ Successfully compiled live state!");

                    // Use the universal translator to dump readable YAML.
                    let yaml_output = mflash_core::translator::to_yaml(&deck)
                        .expect("❌ Failed to translate to YAML");

                    println!("\n--- DECK YAML DUMP ---");
                    println!("{}", yaml_output);
                    println!("----------------------\n");
                }
                Err(e) => {
                    eprintln!("❌ Failed to read live database: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Optimize { workspace_id } => {
            let home = dirs::home_dir().expect("Could not find home directory");
            let workspace_dir = home.join(".mflash_cache").join(&workspace_id);

            println!("🧰 Optimizing media for workspace: {}", workspace_id);

            let conn = mflash_core::db::init_workspace_db(&workspace_dir)
                .expect("❌ Failed to connect to workspace database");

            if let Err(e) = mflash_core::optimizer::optimize_media(&workspace_dir, &conn) {
                eprintln!("❌ Failed to optimize media: {}", e);
                std::process::exit(1);
            }
        }
    }
}