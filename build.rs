use std::io::Result;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=proto/mflash-v4.proto");
    
    // Configure prost to add Serde derives to all generated structs!
    let mut config = prost_build::Config::new();
    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
    
    config.compile_protos(&["proto/mflash-v4.proto"], &["proto/"])?;
    
    Ok(())
}