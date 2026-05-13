// src/lib.rs

/// The generated Protobuf structs for mflash v4.
pub mod pb {
    // prost_build automatically names the output file based on the
    // `package mflash.v4;` declaration inside your .proto file.
    include!(concat!(env!("OUT_DIR"), "/mflash.v4.rs"));
}

pub mod workspace {
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};

    use walkdir::WalkDir;
    use zip::write::FileOptions;
    use zip::{CompressionMethod, ZipArchive, ZipWriter};

    /// Unpacks an .mflash ZIP archive into:
    ///
    /// ~/.mflash_cache/<workspace_id>/
    ///
    /// Uses `enclosed_name()` to prevent path traversal attacks such as:
    ///
    /// ../../dangerous_file
    pub fn unpack_deck(
        archive_path: &Path,
        workspace_id: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        // 1. Resolve the cache directory.
        let home = dirs::home_dir().ok_or("Could not find home directory")?;
        let workspace_dir = home.join(".mflash_cache").join(workspace_id);

        // Make sure the workspace root exists before writing files into it.
        fs::create_dir_all(&workspace_dir)?;

        // 2. Open the .mflash file.
        let file = fs::File::open(archive_path)?;
        let mut archive = ZipArchive::new(file)?;

        println!("📂 Creating workspace at: {}", workspace_dir.display());

        // 3. Extract contents safely.
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;

            // SECURITY:
            // `enclosed_name()` prevents path traversal attacks.
            // For example, entries like `../../somewhere_else`
            // will be rejected.
            let outpath = match file.enclosed_name() {
                Some(path) => workspace_dir.join(path),
                None => {
                    println!("⚠️ Warning: Skipping suspicious file path.");
                    continue;
                }
            };

            if file.is_dir() {
                fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    fs::create_dir_all(parent)?;
                }

                let mut outfile = fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }

        Ok(workspace_dir)
    }

    /// Packs ~/.mflash_cache/<workspace_id>/ into a Zstandard-compressed .mflash archive.
    pub fn pack_deck(
        workspace_id: &str,
        output_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Resolve the cache directory.
        let home = dirs::home_dir().ok_or("Could not find home directory")?;
        let workspace_dir = home.join(".mflash_cache").join(workspace_id);

        if !workspace_dir.exists() {
            return Err("Workspace does not exist. Did you unpack it first?".into());
        }

        // 2. Set up the ZIP file with Zstandard compression.
        let file = fs::File::create(output_path)?;
        let mut zip = ZipWriter::new(file);

        let options = FileOptions::default()
            .compression_method(CompressionMethod::Zstd)
            .unix_permissions(0o755);

        let mut buffer = Vec::new();

        println!("🗜️ Compressing workspace with Zstandard...");
        println!("📦 Source workspace: {}", workspace_dir.display());
        println!("💾 Output archive: {}", output_path.display());

        // 3. Walk the directory and add files/directories to the archive.
        for entry in WalkDir::new(&workspace_dir) {
            let entry = entry?;
            let path = entry.path();
            let name = path.strip_prefix(&workspace_dir)?;

            // Skip the root folder itself.
            if name.as_os_str().is_empty() {
                continue;
            }

            // Standardize paths to forward slashes for cross-platform ZIP compatibility.
            let name_str = name.to_string_lossy().replace('\\', "/");

            if path.is_file() {
                zip.start_file(name_str, options)?;

                let mut f = fs::File::open(path)?;
                f.read_to_end(&mut buffer)?;
                zip.write_all(&buffer)?;
                buffer.clear();
            } else if path.is_dir() {
                zip.add_directory(name_str, options)?;
            }
        }

        zip.finish()?;

        println!("✅ Packed deck successfully.");

        Ok(())
    }
}

pub mod schema {
    use crate::pb::Deck;
    use bytes::Bytes;
    use prost::Message;
    use std::fs;
    use std::path::Path;

    /// Reads a deck.pb file and returns the Deck struct.
    pub fn read_deck(workspace_dir: &Path) -> Result<Deck, Box<dyn std::error::Error>> {
        let pb_path = workspace_dir.join("deck.pb");

        if !pb_path.exists() {
            return Err("deck.pb not found in workspace. Is this a valid v4 deck?".into());
        }

        let data = fs::read(pb_path)?;
        let deck = Deck::decode(Bytes::from(data))?;

        Ok(deck)
    }

    /// Serializes a Deck struct and writes it to deck.pb.
    pub fn write_deck(
        workspace_dir: &Path,
        deck: &Deck,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let pb_path = workspace_dir.join("deck.pb");

        let mut buf = Vec::with_capacity(deck.encoded_len());
        deck.encode(&mut buf)?;

        fs::write(pb_path, buf)?;

        Ok(())
    }
}

pub mod db {
    use crate::pb::{Card, Deck};
    use rusqlite::{Connection, Result};
    use std::path::Path;

    /// Initializes the SQLite database for live editing.
    pub fn init_workspace_db(workspace_dir: &Path) -> Result<Connection> {
        let db_path = workspace_dir.join("workspace.db");
        let conn = Connection::open(&db_path)?;

        // Enable WAL mode for fast, safe live editing writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // Create the working tables.
        conn.execute_batch(
            "BEGIN;

            -- Deck Metadata
            CREATE TABLE IF NOT EXISTS deck_meta (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                version INTEGER NOT NULL,
                default_term_lang TEXT,
                default_def_lang TEXT
            );

            -- The Cards
            CREATE TABLE IF NOT EXISTS cards (
                id TEXT PRIMARY KEY,
                kind INTEGER NOT NULL,
                term TEXT,
                definition TEXT,
                prompt TEXT,
                answer TEXT,
                media TEXT
            );

            -- We will add tables for Media, Extensions, and Occlusion Masks later!

            COMMIT;"
        )?;

        // Existing workspaces may already have a `cards` table from before `media` existed.
        ensure_cards_media_column(&conn)?;

        println!("🗄️  Initialized SQLite workspace database.");

        Ok(conn)
    }

    /// Adds `cards.media` to older workspace databases if it does not already exist.
    fn ensure_cards_media_column(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(cards)")?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

        let mut has_media_column = false;
        for column in columns {
            if column? == "media" {
                has_media_column = true;
                break;
            }
        }

        if !has_media_column {
            conn.execute("ALTER TABLE cards ADD COLUMN media TEXT", [])?;
            println!("🧱 Migrated cards table: added media column.");
        }

        Ok(())
    }

    /// Reads a Protobuf Deck and securely inserts all cards into SQLite using a high-speed transaction.
    pub fn import_pb_to_db(conn: &mut Connection, deck: &Deck) -> Result<()> {
        // Start a transaction for massive performance gains.
        let tx = conn.transaction()?;

        // 1. Insert deck metadata.
        tx.execute(
            "INSERT OR REPLACE INTO deck_meta (id, title, version, default_term_lang, default_def_lang)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                &deck.id,
                &deck.title,
                &deck.version,
                &deck.default_term_lang,
                &deck.default_def_lang,
            ),
        )?;

        // 2. Prepare the card statement once, then loop.
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO cards (id, kind, term, definition, prompt, answer)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;

            for card in &deck.cards {
                stmt.execute((
                    &card.id,
                    card.kind,
                    &card.term,
                    &card.definition,
                    &card.prompt,
                    &card.answer,
                ))?;
            }
        }

        // Commit the transaction to disk.
        tx.commit()?;

        println!(
            "⚡ Synced {} cards into live SQLite workspace.",
            deck.cards.len()
        );

        Ok(())
    }

    /// Reads the SQLite tables and compiles them back into a Protobuf Deck.
    pub fn export_db_to_pb(conn: &Connection) -> Result<Deck> {
        let mut deck = Deck::default();
        deck.format = "mflash".to_string();

        // 1. Pull deck metadata.
        conn.query_row(
            "SELECT id, title, version, default_term_lang, default_def_lang FROM deck_meta LIMIT 1",
            [],
            |row| {
                deck.id = row.get(0)?;
                deck.title = row.get(1)?;
                deck.version = row.get(2)?;
                deck.default_term_lang = row.get(3)?;
                deck.default_def_lang = row.get(4)?;
                Ok(())
            },
        )?;

        // 2. Pull all cards.
        let mut stmt = conn.prepare("SELECT id, kind, term, definition, prompt, answer FROM cards")?;
        let card_iter = stmt.query_map([], |row| {
            let mut card = Card::default();
            card.id = row.get(0)?;
            card.kind = row.get(1)?;
            card.term = row.get(2)?;
            card.definition = row.get(3)?;
            card.prompt = row.get(4)?;
            card.answer = row.get(5)?;
            Ok(card)
        })?;

        for card in card_iter {
            deck.cards.push(card?);
        }

        println!(
            "⚡ Compiled {} cards from SQLite to Protobuf structure.",
            deck.cards.len()
        );

        Ok(deck)
    }
}

pub mod translator {
    use crate::pb::Deck;

    /// Formats the deck as pretty-printed JSON.
    pub fn to_json(deck: &Deck) -> Result<String, Box<dyn std::error::Error>> {
        let json_string = serde_json::to_string_pretty(deck)?;
        Ok(json_string)
    }

    /// Formats the deck as TOML.
    pub fn to_toml(deck: &Deck) -> Result<String, Box<dyn std::error::Error>> {
        let toml_string = toml::to_string(deck)?;
        Ok(toml_string)
    }

    /// Formats the deck as YAML.
    pub fn to_yaml(deck: &Deck) -> Result<String, Box<dyn std::error::Error>> {
        let yaml_string = serde_yaml::to_string(deck)?;
        Ok(yaml_string)
    }
}

pub mod optimizer {
    use rayon::prelude::*;
    use rusqlite::Connection;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    /// Scans the workspace, compresses images in parallel, and updates the database.
    pub fn optimize_media(
        workspace_dir: &Path,
        conn: &Connection,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let assets_dir = workspace_dir.join("assets");

        if !assets_dir.exists() {
            println!("ℹ️ No assets folder found. Nothing to optimize.");
            return Ok(());
        }

        println!("🔍 Scanning for unoptimized media...");

        // 1. Collect all image files first.
        // This gives Rayon a clean Vec to split across worker threads.
        let mut image_files: Vec<PathBuf> = Vec::new();

        for entry in WalkDir::new(&assets_dir) {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext = ext.to_lowercase();

                    // Look for heavier formats that are worth converting.
                    if ext == "png" || ext == "jpg" || ext == "jpeg" {
                        image_files.push(path.to_path_buf());
                    }
                }
            }
        }

        if image_files.is_empty() {
            println!("✨ All media is already optimized!");
            return Ok(());
        }

        println!(
            "⚙️ Found {} images. Firing up all CPU cores for compression...",
            image_files.len()
        );

        // 2. THE RAYON MAGIC:
        // Process image conversions in parallel across CPU cores.
        let conversion_results: Vec<(String, String)> = image_files
            .into_par_iter()
            .filter_map(|old_path| {
                // Calculate the new path by changing the extension to .webp.
                let mut new_path = old_path.clone();
                new_path.set_extension("webp");

                // Open the heavy image.
                if let Ok(img) = image::open(&old_path) {
                    // Save it as WebP.
                    if img
                        .save_with_format(&new_path, image::ImageFormat::WebP)
                        .is_ok()
                    {
                        // Calculate workspace-relative paths for the database.
                        let old_relative = old_path
                            .strip_prefix(workspace_dir)
                            .ok()?
                            .to_string_lossy()
                            .replace('\\', "/");

                        let new_relative = new_path
                            .strip_prefix(workspace_dir)
                            .ok()?
                            .to_string_lossy()
                            .replace('\\', "/");

                        // Delete the old, heavy file after the WebP has been written.
                        let _ = fs::remove_file(&old_path);

                        // Return the mapping so SQLite can be updated sequentially later.
                        return Some((old_relative, new_relative));
                    }
                }

                None
            })
            .collect();

        // 3. Update SQLite sequentially after the parallel file work.
        // This avoids database write locks while all CPU cores are crushing images.
        for (old_src, new_src) in conversion_results {
            conn.execute(
                "UPDATE cards
                 SET media = REPLACE(media, ?1, ?2)
                 WHERE media LIKE '%' || ?1 || '%'",
                (&old_src, &new_src),
            )?;

            println!("   📉 Crushed: {} -> {}", old_src, new_src);
        }

        println!("✅ Media optimization complete!");

        Ok(())
    }
}
